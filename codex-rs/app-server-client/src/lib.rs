mod remote;

use std::error::Error;
use std::fmt;
use std::io::Error as IoError;
use std::io::ErrorKind;
use std::io::Result as IoResult;
use std::sync::Arc;
use std::time::Duration;

pub use codex_app_server::app_server_control_socket_path;
pub use codex_app_server::in_process::DEFAULT_IN_PROCESS_CHANNEL_CAPACITY;
pub use codex_app_server::in_process::InProcessServerEvent;
use codex_app_server::in_process::InProcessStartArgs;
use codex_app_server::in_process::LogDbLayer;
pub use codex_app_server::in_process::StateDbHandle;
use codex_app_server_protocol::ClientInfo;
use codex_app_server_protocol::ClientNotification;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::ConfigWarningNotification;
use codex_app_server_protocol::InitializeCapabilities;
use codex_app_server_protocol::InitializeParams;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::Result as JsonRpcResult;
use codex_app_server_protocol::ServerNotification;
use codex_app_server_protocol::ServerRequest;
use codex_arg0::Arg0DispatchPaths;
use codex_config::LoaderOverrides;
use codex_config::NoopThreadConfigLoader;
use codex_config::ThreadConfigLoader;
use codex_core::config::Config;
use codex_protocol::protocol::SessionSource;
use serde::de::DeserializeOwned;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::time::timeout;
use toml::Value as TomlValue;
use tracing::warn;

pub use crate::remote::RemoteAppServerClient;
pub use crate::remote::RemoteAppServerConnectArgs;
pub use crate::remote::RemoteAppServerEndpoint;

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

pub type RequestResult = std::result::Result<JsonRpcResult, JSONRPCErrorError>;

#[derive(Debug, Clone)]
pub enum AppServerEvent {
    Lagged { skipped: usize },
    ServerNotification(ServerNotification),
    ServerRequest(ServerRequest),
    Disconnected { message: String },
}

impl From<InProcessServerEvent> for AppServerEvent {
    fn from(value: InProcessServerEvent) -> Self {
        match value {
            InProcessServerEvent::Lagged { skipped } => Self::Lagged { skipped },
            InProcessServerEvent::ServerNotification(notification) => {
                Self::ServerNotification(notification)
            }
            InProcessServerEvent::ServerRequest(request) => Self::ServerRequest(request),
        }
    }
}

fn event_requires_delivery(event: &InProcessServerEvent) -> bool {
    match event {
        InProcessServerEvent::ServerNotification(notification) => {
            server_notification_requires_delivery(notification)
        }
        _ => false,
    }
}

pub(crate) fn server_notification_requires_delivery(notification: &ServerNotification) -> bool {
    matches!(
        notification,
        ServerNotification::TurnCompleted(_)
            | ServerNotification::ThreadSettingsUpdated(_)
            | ServerNotification::ItemCompleted(_)
            | ServerNotification::AgentMessageDelta(_)
            | ServerNotification::ReasoningSummaryTextDelta(_)
            | ServerNotification::ReasoningTextDelta(_)
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ForwardEventResult {
    Continue,

    DisableStream,
}

async fn forward_in_process_event<F>(
    event_tx: &mpsc::Sender<InProcessServerEvent>,
    skipped_events: &mut usize,
    event: InProcessServerEvent,
    mut reject_server_request: F,
) -> ForwardEventResult
where
    F: FnMut(ServerRequest),
{
    if *skipped_events > 0 {
        if event_requires_delivery(&event) {
            if event_tx
                .send(InProcessServerEvent::Lagged {
                    skipped: *skipped_events,
                })
                .await
                .is_err()
            {
                return ForwardEventResult::DisableStream;
            }
            *skipped_events = 0;
        } else {
            match event_tx.try_send(InProcessServerEvent::Lagged {
                skipped: *skipped_events,
            }) {
                Ok(()) => {
                    *skipped_events = 0;
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    *skipped_events = skipped_events.saturating_add(1);
                    warn!("dropping in-process app-server event because consumer queue is full");
                    if let InProcessServerEvent::ServerRequest(request) = event {
                        reject_server_request(request);
                    }
                    return ForwardEventResult::Continue;
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    return ForwardEventResult::DisableStream;
                }
            }
        }
    }

    if event_requires_delivery(&event) {
        if event_tx.send(event).await.is_err() {
            return ForwardEventResult::DisableStream;
        }
        return ForwardEventResult::Continue;
    }

    match event_tx.try_send(event) {
        Ok(()) => ForwardEventResult::Continue,
        Err(mpsc::error::TrySendError::Full(event)) => {
            *skipped_events = skipped_events.saturating_add(1);
            warn!("dropping in-process app-server event because consumer queue is full");
            if let InProcessServerEvent::ServerRequest(request) = event {
                reject_server_request(request);
            }
            ForwardEventResult::Continue
        }
        Err(mpsc::error::TrySendError::Closed(_)) => ForwardEventResult::DisableStream,
    }
}

#[derive(Debug)]
pub enum TypedRequestError {
    Transport {
        method: String,
        source: IoError,
    },
    Server {
        method: String,
        source: JSONRPCErrorError,
    },
    Deserialize {
        method: String,
        source: serde_json::Error,
    },
}

impl fmt::Display for TypedRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport { method, source } => {
                write!(f, "{method} transport error: {source}")
            }
            Self::Server { method, source } => {
                write!(
                    f,
                    "{method} failed: {} (code {})",
                    source.message, source.code
                )?;
                if let Some(data) = source.data.as_ref() {
                    write!(f, ", data: {data}")?;
                }
                Ok(())
            }
            Self::Deserialize { method, source } => {
                write!(f, "{method} response decode error: {source}")
            }
        }
    }
}

impl Error for TypedRequestError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Transport { source, .. } => Some(source),
            Self::Server { .. } => None,
            Self::Deserialize { source, .. } => Some(source),
        }
    }
}

#[derive(Clone)]
pub struct InProcessClientStartArgs {
    pub arg0_paths: Arg0DispatchPaths,

    pub config: Arc<Config>,

    pub cli_overrides: Vec<(String, TomlValue)>,

    pub loader_overrides: LoaderOverrides,

    pub strict_config: bool,

    pub log_db: Option<LogDbLayer>,

    pub state_db: Option<StateDbHandle>,

    pub config_warnings: Vec<ConfigWarningNotification>,

    pub session_source: SessionSource,

    pub enable_codex_api_key_env: bool,

    pub client_name: String,

    pub client_version: String,

    pub opt_out_notification_methods: Vec<String>,

    pub channel_capacity: usize,
}

fn configured_thread_config_loader() -> Arc<dyn ThreadConfigLoader> {
    Arc::new(NoopThreadConfigLoader)
}

impl InProcessClientStartArgs {
    pub fn initialize_params(&self) -> InitializeParams {
        let capabilities = InitializeCapabilities {
            opt_out_notification_methods: if self.opt_out_notification_methods.is_empty() {
                None
            } else {
                Some(self.opt_out_notification_methods.clone())
            },
        };

        InitializeParams {
            client_info: ClientInfo {
                name: self.client_name.clone(),
                title: None,
                version: self.client_version.clone(),
            },
            capabilities: Some(capabilities),
        }
    }

    fn into_runtime_start_args(self) -> InProcessStartArgs {
        let initialize = self.initialize_params();
        let thread_config_loader = configured_thread_config_loader();
        InProcessStartArgs {
            arg0_paths: self.arg0_paths,
            config: self.config,
            cli_overrides: self.cli_overrides,
            loader_overrides: self.loader_overrides,
            strict_config: self.strict_config,
            thread_config_loader,
            log_db: self.log_db,
            state_db: self.state_db,
            config_warnings: self.config_warnings,
            session_source: self.session_source,
            enable_codex_api_key_env: self.enable_codex_api_key_env,
            initialize,
            channel_capacity: self.channel_capacity,
        }
    }
}

enum ClientCommand {
    Request {
        request: Box<ClientRequest>,
        response_tx: oneshot::Sender<IoResult<RequestResult>>,
    },
    Notify {
        notification: ClientNotification,
        response_tx: oneshot::Sender<IoResult<()>>,
    },
    ResolveServerRequest {
        request_id: RequestId,
        result: JsonRpcResult,
        response_tx: oneshot::Sender<IoResult<()>>,
    },
    RejectServerRequest {
        request_id: RequestId,
        error: JSONRPCErrorError,
        response_tx: oneshot::Sender<IoResult<()>>,
    },
    Shutdown {
        response_tx: oneshot::Sender<IoResult<()>>,
    },
}

pub struct InProcessAppServerClient {
    command_tx: mpsc::Sender<ClientCommand>,
    event_rx: mpsc::Receiver<InProcessServerEvent>,
    worker_handle: tokio::task::JoinHandle<()>,
}

#[derive(Clone)]
pub struct InProcessAppServerRequestHandle {
    command_tx: mpsc::Sender<ClientCommand>,
}

#[derive(Clone)]
pub enum AppServerRequestHandle {
    InProcess(InProcessAppServerRequestHandle),
    Remote(crate::remote::RemoteAppServerRequestHandle),
}

pub enum AppServerClient {
    InProcess(InProcessAppServerClient),
    Remote(RemoteAppServerClient),
}

impl InProcessAppServerClient {
    pub async fn start(args: InProcessClientStartArgs) -> IoResult<Self> {
        let channel_capacity = args.channel_capacity.max(1);
        let mut handle =
            codex_app_server::in_process::start(args.into_runtime_start_args()).await?;
        let request_sender = handle.sender();
        let (command_tx, mut command_rx) = mpsc::channel::<ClientCommand>(channel_capacity);
        let (event_tx, event_rx) = mpsc::channel::<InProcessServerEvent>(channel_capacity);

        let worker_handle = tokio::spawn(async move {
            let mut event_stream_enabled = true;
            let mut skipped_events = 0usize;
            loop {
                tokio::select! {
                    command = command_rx.recv() => {
                        match command {
                            Some(ClientCommand::Request { request, response_tx }) => {
                                let request_sender = request_sender.clone();



                                tokio::spawn(async move {
                                    let result = request_sender.request(*request).await;
                                    let _ = response_tx.send(result);
                                });
                            }
                            Some(ClientCommand::Notify {
                                notification,
                                response_tx,
                            }) => {
                                let result = request_sender.notify(notification);
                                let _ = response_tx.send(result);
                            }
                            Some(ClientCommand::ResolveServerRequest {
                                request_id,
                                result,
                                response_tx,
                            }) => {
                                let send_result =
                                    request_sender.respond_to_server_request(request_id, result);
                                let _ = response_tx.send(send_result);
                            }
                            Some(ClientCommand::RejectServerRequest {
                                request_id,
                                error,
                                response_tx,
                            }) => {
                                let send_result = request_sender.fail_server_request(request_id, error);
                                let _ = response_tx.send(send_result);
                            }
                            Some(ClientCommand::Shutdown { response_tx }) => {
                                let shutdown_result = handle.shutdown().await;
                                let _ = response_tx.send(shutdown_result);
                                break;
                            }
                            None => {
                                let _ = handle.shutdown().await;
                                break;
                            }
                        }
                    }
                    event = handle.next_event(), if event_stream_enabled => {
                        let Some(event) = event else {
                            break;
                        };
                        if let InProcessServerEvent::ServerRequest(
                            ServerRequest::ChatgptAuthTokensRefresh { request_id, .. }
                        ) = &event
                        {
                            let send_result = request_sender.fail_server_request(
                                request_id.clone(),
                                JSONRPCErrorError {
                                    code: -32000,
                                    message: "chatgpt auth token refresh is not supported for in-process app-server clients".to_string(),
                                    data: None,
                                },
                            );
                            if let Err(err) = send_result {
                                warn!(
                                    "failed to reject unsupported chatgpt auth token refresh request: {err}"
                                );
                            }
                            continue;
                        }

                        match forward_in_process_event(
                            &event_tx,
                            &mut skipped_events,
                            event,
                            |request| {
                                let _ = request_sender.fail_server_request(
                                    request.id().clone(),
                                    JSONRPCErrorError {
                                        code: -32001,
                                        message: "in-process app-server event queue is full"
                                            .to_string(),
                                        data: None,
                                    },
                                );
                            },
                        )
                        .await
                        {
                            ForwardEventResult::Continue => {}
                            ForwardEventResult::DisableStream => {
                                event_stream_enabled = false;
                            }
                        }
                    }
                }
            }
        });

        Ok(Self {
            command_tx,
            event_rx,
            worker_handle,
        })
    }

    pub fn request_handle(&self) -> InProcessAppServerRequestHandle {
        InProcessAppServerRequestHandle {
            command_tx: self.command_tx.clone(),
        }
    }

    pub async fn request(&self, request: ClientRequest) -> IoResult<RequestResult> {
        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx
            .send(ClientCommand::Request {
                request: Box::new(request),
                response_tx,
            })
            .await
            .map_err(|_| {
                IoError::new(
                    ErrorKind::BrokenPipe,
                    "in-process app-server worker channel is closed",
                )
            })?;
        response_rx.await.map_err(|_| {
            IoError::new(
                ErrorKind::BrokenPipe,
                "in-process app-server request channel is closed",
            )
        })?
    }

    pub async fn request_typed<T>(&self, request: ClientRequest) -> Result<T, TypedRequestError>
    where
        T: DeserializeOwned,
    {
        let method = request_method_name(&request);
        let response =
            self.request(request)
                .await
                .map_err(|source| TypedRequestError::Transport {
                    method: method.clone(),
                    source,
                })?;
        let result = response.map_err(|source| TypedRequestError::Server {
            method: method.clone(),
            source,
        })?;
        serde_json::from_value(result)
            .map_err(|source| TypedRequestError::Deserialize { method, source })
    }

    pub async fn notify(&self, notification: ClientNotification) -> IoResult<()> {
        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx
            .send(ClientCommand::Notify {
                notification,
                response_tx,
            })
            .await
            .map_err(|_| {
                IoError::new(
                    ErrorKind::BrokenPipe,
                    "in-process app-server worker channel is closed",
                )
            })?;
        response_rx.await.map_err(|_| {
            IoError::new(
                ErrorKind::BrokenPipe,
                "in-process app-server notify channel is closed",
            )
        })?
    }

    pub async fn resolve_server_request(
        &self,
        request_id: RequestId,
        result: JsonRpcResult,
    ) -> IoResult<()> {
        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx
            .send(ClientCommand::ResolveServerRequest {
                request_id,
                result,
                response_tx,
            })
            .await
            .map_err(|_| {
                IoError::new(
                    ErrorKind::BrokenPipe,
                    "in-process app-server worker channel is closed",
                )
            })?;
        response_rx.await.map_err(|_| {
            IoError::new(
                ErrorKind::BrokenPipe,
                "in-process app-server resolve channel is closed",
            )
        })?
    }

    pub async fn reject_server_request(
        &self,
        request_id: RequestId,
        error: JSONRPCErrorError,
    ) -> IoResult<()> {
        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx
            .send(ClientCommand::RejectServerRequest {
                request_id,
                error,
                response_tx,
            })
            .await
            .map_err(|_| {
                IoError::new(
                    ErrorKind::BrokenPipe,
                    "in-process app-server worker channel is closed",
                )
            })?;
        response_rx.await.map_err(|_| {
            IoError::new(
                ErrorKind::BrokenPipe,
                "in-process app-server reject channel is closed",
            )
        })?
    }

    pub async fn next_event(&mut self) -> Option<InProcessServerEvent> {
        self.event_rx.recv().await
    }

    pub async fn shutdown(self) -> IoResult<()> {
        let Self {
            command_tx,
            event_rx,
            worker_handle,
        } = self;
        let mut worker_handle = worker_handle;

        drop(event_rx);
        let (response_tx, response_rx) = oneshot::channel();
        if command_tx
            .send(ClientCommand::Shutdown { response_tx })
            .await
            .is_ok()
            && let Ok(command_result) = timeout(SHUTDOWN_TIMEOUT, response_rx).await
        {
            command_result.map_err(|_| {
                IoError::new(
                    ErrorKind::BrokenPipe,
                    "in-process app-server shutdown channel is closed",
                )
            })??;
        }

        if let Err(_elapsed) = timeout(SHUTDOWN_TIMEOUT, &mut worker_handle).await {
            worker_handle.abort();
            let _ = worker_handle.await;
        }
        Ok(())
    }
}

impl InProcessAppServerRequestHandle {
    pub async fn request(&self, request: ClientRequest) -> IoResult<RequestResult> {
        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx
            .send(ClientCommand::Request {
                request: Box::new(request),
                response_tx,
            })
            .await
            .map_err(|_| {
                IoError::new(
                    ErrorKind::BrokenPipe,
                    "in-process app-server worker channel is closed",
                )
            })?;
        response_rx.await.map_err(|_| {
            IoError::new(
                ErrorKind::BrokenPipe,
                "in-process app-server request channel is closed",
            )
        })?
    }

    pub async fn request_typed<T>(&self, request: ClientRequest) -> Result<T, TypedRequestError>
    where
        T: DeserializeOwned,
    {
        let method = request_method_name(&request);
        let response =
            self.request(request)
                .await
                .map_err(|source| TypedRequestError::Transport {
                    method: method.clone(),
                    source,
                })?;
        let result = response.map_err(|source| TypedRequestError::Server {
            method: method.clone(),
            source,
        })?;
        serde_json::from_value(result)
            .map_err(|source| TypedRequestError::Deserialize { method, source })
    }
}

impl AppServerRequestHandle {
    pub async fn request(&self, request: ClientRequest) -> IoResult<RequestResult> {
        match self {
            Self::InProcess(handle) => handle.request(request).await,
            Self::Remote(handle) => handle.request(request).await,
        }
    }

    pub async fn request_typed<T>(&self, request: ClientRequest) -> Result<T, TypedRequestError>
    where
        T: DeserializeOwned,
    {
        match self {
            Self::InProcess(handle) => handle.request_typed(request).await,
            Self::Remote(handle) => handle.request_typed(request).await,
        }
    }
}

impl AppServerClient {
    pub async fn request(&self, request: ClientRequest) -> IoResult<RequestResult> {
        match self {
            Self::InProcess(client) => client.request(request).await,
            Self::Remote(client) => client.request(request).await,
        }
    }

    pub async fn request_typed<T>(&self, request: ClientRequest) -> Result<T, TypedRequestError>
    where
        T: DeserializeOwned,
    {
        match self {
            Self::InProcess(client) => client.request_typed(request).await,
            Self::Remote(client) => client.request_typed(request).await,
        }
    }

    pub async fn notify(&self, notification: ClientNotification) -> IoResult<()> {
        match self {
            Self::InProcess(client) => client.notify(notification).await,
            Self::Remote(client) => client.notify(notification).await,
        }
    }

    pub async fn resolve_server_request(
        &self,
        request_id: RequestId,
        result: JsonRpcResult,
    ) -> IoResult<()> {
        match self {
            Self::InProcess(client) => client.resolve_server_request(request_id, result).await,
            Self::Remote(client) => client.resolve_server_request(request_id, result).await,
        }
    }

    pub async fn reject_server_request(
        &self,
        request_id: RequestId,
        error: JSONRPCErrorError,
    ) -> IoResult<()> {
        match self {
            Self::InProcess(client) => client.reject_server_request(request_id, error).await,
            Self::Remote(client) => client.reject_server_request(request_id, error).await,
        }
    }

    pub async fn next_event(&mut self) -> Option<AppServerEvent> {
        match self {
            Self::InProcess(client) => client.next_event().await.map(Into::into),
            Self::Remote(client) => client.next_event().await,
        }
    }

    pub async fn shutdown(self) -> IoResult<()> {
        match self {
            Self::InProcess(client) => client.shutdown().await,
            Self::Remote(client) => client.shutdown().await,
        }
    }

    pub fn request_handle(&self) -> AppServerRequestHandle {
        match self {
            Self::InProcess(client) => AppServerRequestHandle::InProcess(client.request_handle()),
            Self::Remote(client) => AppServerRequestHandle::Remote(client.request_handle()),
        }
    }
}

pub(crate) fn request_method_name(request: &ClientRequest) -> String {
    serde_json::to_value(request)
        .ok()
        .and_then(|value| {
            value
                .get("method")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "<unknown>".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_app_server_protocol::GetAccountResponse;
    use codex_app_server_protocol::JSONRPCMessage;
    use codex_app_server_protocol::JSONRPCResponse;
    use codex_app_server_protocol::ServerNotification;
    use codex_app_server_protocol::SessionSource as ApiSessionSource;
    use codex_app_server_protocol::ThreadStartParams;
    use codex_app_server_protocol::ThreadStartResponse;
    use codex_core::config::ConfigBuilder;
    use codex_core::init_state_db;
    use codex_uds::UnixListener;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use futures::SinkExt;
    use futures::StreamExt;
    use pretty_assertions::assert_eq;
    use std::ops::Deref;
    use std::path::Path;
    use tempfile::TempDir;
    use tokio::time::Duration;
    use tokio::time::timeout;
    use tokio_tungstenite::accept_async;
    use tokio_tungstenite::tungstenite::Message;

    async fn build_test_config_for_codex_home(codex_home: &Path) -> Config {
        match ConfigBuilder::default()
            .codex_home(codex_home.to_path_buf())
            .build()
            .await
        {
            Ok(config) => config,
            Err(_) => Config::load_default_with_cli_overrides_for_codex_home(
                codex_home.to_path_buf(),
                Vec::new(),
            )
            .await
            .expect("default config should load"),
        }
    }

    struct TestClient {
        _codex_home: TempDir,
        client: InProcessAppServerClient,
    }

    impl Deref for TestClient {
        type Target = InProcessAppServerClient;

        fn deref(&self) -> &Self::Target {
            &self.client
        }
    }

    impl TestClient {
        async fn shutdown(self) -> IoResult<()> {
            self.client.shutdown().await
        }
    }

    async fn start_test_client_with_capacity(
        session_source: SessionSource,
        channel_capacity: usize,
    ) -> TestClient {
        let codex_home = TempDir::new().expect("temp dir");
        let config = Arc::new(build_test_config_for_codex_home(codex_home.path()).await);
        let state_db = init_state_db(config.as_ref())
            .await
            .expect("state db should initialize for in-process test");
        let client = InProcessAppServerClient::start(InProcessClientStartArgs {
            arg0_paths: Arg0DispatchPaths::default(),
            config,
            cli_overrides: Vec::new(),
            loader_overrides: LoaderOverrides::default(),
            strict_config: false,
            log_db: None,
            state_db: Some(state_db),
            config_warnings: Vec::new(),
            session_source,
            enable_codex_api_key_env: false,
            client_name: "codex-app-server-client-test".to_string(),
            client_version: "0.0.0-test".to_string(),
            opt_out_notification_methods: Vec::new(),
            channel_capacity,
        })
        .await
        .expect("in-process app-server client should start");

        TestClient {
            _codex_home: codex_home,
            client,
        }
    }

    async fn start_test_client(session_source: SessionSource) -> TestClient {
        start_test_client_with_capacity(session_source, DEFAULT_IN_PROCESS_CHANNEL_CAPACITY).await
    }
    async fn expect_remote_initialize<S>(websocket: &mut tokio_tungstenite::WebSocketStream<S>)
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let JSONRPCMessage::Request(request) = read_websocket_message(websocket).await else {
            panic!("expected initialize request");
        };
        assert_eq!(request.method, "initialize");
        write_websocket_message(
            websocket,
            JSONRPCMessage::Response(JSONRPCResponse {
                id: request.id,
                result: serde_json::json!({}),
            }),
        )
        .await;

        let JSONRPCMessage::Notification(notification) = read_websocket_message(websocket).await
        else {
            panic!("expected initialized notification");
        };
        assert_eq!(notification.method, "initialized");
    }

    async fn read_websocket_message<S>(
        websocket: &mut tokio_tungstenite::WebSocketStream<S>,
    ) -> JSONRPCMessage
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        loop {
            let frame = websocket
                .next()
                .await
                .expect("frame should be available")
                .expect("frame should decode");
            match frame {
                Message::Text(text) => {
                    return serde_json::from_str::<JSONRPCMessage>(&text)
                        .expect("text frame should be valid JSON-RPC");
                }
                Message::Binary(_) | Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {
                    continue;
                }
                Message::Close(_) => panic!("unexpected close frame"),
            }
        }
    }

    async fn write_websocket_message<S>(
        websocket: &mut tokio_tungstenite::WebSocketStream<S>,
        message: JSONRPCMessage,
    ) where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        websocket
            .send(Message::Text(
                serde_json::to_string(&message)
                    .expect("message should serialize")
                    .into(),
            ))
            .await
            .expect("message should send");
    }

    fn command_execution_output_delta_notification(delta: &str) -> ServerNotification {
        ServerNotification::CommandExecutionOutputDelta(
            codex_app_server_protocol::CommandExecutionOutputDeltaNotification {
                thread_id: "thread".to_string(),
                turn_id: "turn".to_string(),
                item_id: "item".to_string(),
                delta: delta.to_string(),
            },
        )
    }

    fn agent_message_delta_notification(delta: &str) -> ServerNotification {
        ServerNotification::AgentMessageDelta(
            codex_app_server_protocol::AgentMessageDeltaNotification {
                thread_id: "thread".to_string(),
                turn_id: "turn".to_string(),
                item_id: "item".to_string(),
                delta: delta.to_string(),
            },
        )
    }

    fn item_completed_notification(text: &str) -> ServerNotification {
        ServerNotification::ItemCompleted(codex_app_server_protocol::ItemCompletedNotification {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            completed_at_ms: 0,
            item: codex_app_server_protocol::ThreadItem::AgentMessage {
                id: "item".to_string(),
                text: text.to_string(),
                phase: None,
            },
        })
    }

    fn turn_completed_notification() -> ServerNotification {
        ServerNotification::TurnCompleted(codex_app_server_protocol::TurnCompletedNotification {
            thread_id: "thread".to_string(),
            turn: codex_app_server_protocol::Turn {
                id: "turn".to_string(),
                items_view: codex_app_server_protocol::TurnItemsView::Full,
                items: Vec::new(),
                status: codex_app_server_protocol::TurnStatus::Completed,
                error: None,
                started_at: None,
                completed_at: Some(0),
                duration_ms: Some(1),
            },
        })
    }
    #[tokio::test]
    async fn typed_request_reports_json_rpc_errors() {
        let client = start_test_client(SessionSource::Exec).await;
        let err = client
            .request_typed::<codex_app_server_protocol::ThreadReadResponse>(
                ClientRequest::ThreadRead {
                    request_id: RequestId::Integer(99),
                    params: codex_app_server_protocol::ThreadReadParams {
                        thread_id: "missing-thread".to_string(),
                        include_turns: false,
                    },
                },
            )
            .await
            .expect_err("missing thread should return a JSON-RPC error");
        assert!(
            err.to_string().starts_with("thread/read failed:"),
            "expected method-qualified JSON-RPC failure message"
        );
        client.shutdown().await.expect("shutdown should complete");
    }

    #[tokio::test]
    async fn caller_provided_session_source_is_applied() {
        for (session_source, expected_source) in [
            (SessionSource::Exec, ApiSessionSource::Exec),
            (SessionSource::Cli, ApiSessionSource::Cli),
        ] {
            let client = start_test_client(session_source).await;
            let parsed: ThreadStartResponse = client
                .request_typed(ClientRequest::ThreadStart {
                    request_id: RequestId::Integer(2),
                    params: ThreadStartParams {
                        ephemeral: Some(true),
                        ..ThreadStartParams::default()
                    },
                })
                .await
                .expect("thread/start should succeed");
            assert_eq!(parsed.thread.source, expected_source);
            client.shutdown().await.expect("shutdown should complete");
        }
    }

    #[tokio::test]
    async fn threads_started_via_app_server_are_visible_through_typed_requests() {
        let client = start_test_client(SessionSource::Cli).await;

        let response: ThreadStartResponse = client
            .request_typed(ClientRequest::ThreadStart {
                request_id: RequestId::Integer(3),
                params: ThreadStartParams {
                    ephemeral: Some(true),
                    ..ThreadStartParams::default()
                },
            })
            .await
            .expect("thread/start should succeed");
        let read = client
            .request_typed::<codex_app_server_protocol::ThreadReadResponse>(
                ClientRequest::ThreadRead {
                    request_id: RequestId::Integer(4),
                    params: codex_app_server_protocol::ThreadReadParams {
                        thread_id: response.thread.id.clone(),
                        include_turns: false,
                    },
                },
            )
            .await
            .expect("thread/read should return the newly started thread");
        assert_eq!(read.thread.id, response.thread.id);

        client.shutdown().await.expect("shutdown should complete");
    }

    #[tokio::test]
    async fn tiny_channel_capacity_still_supports_request_roundtrip() {
        let client = start_test_client_with_capacity(SessionSource::Exec, 1).await;
        let _response: ThreadStartResponse = client
            .request_typed(ClientRequest::ThreadStart {
                request_id: RequestId::Integer(1),
                params: ThreadStartParams {
                    ephemeral: Some(true),
                    ..ThreadStartParams::default()
                },
            })
            .await
            .expect("typed request should succeed");
        client.shutdown().await.expect("shutdown should complete");
    }

    #[tokio::test]
    async fn forward_in_process_event_preserves_transcript_notifications_under_backpressure() {
        let (event_tx, mut event_rx) = mpsc::channel(1);
        event_tx
            .send(InProcessServerEvent::ServerNotification(
                command_execution_output_delta_notification("stdout-1"),
            ))
            .await
            .expect("initial event should enqueue");

        let mut skipped_events = 0usize;
        let result = forward_in_process_event(
            &event_tx,
            &mut skipped_events,
            InProcessServerEvent::ServerNotification(command_execution_output_delta_notification(
                "stdout-2",
            )),
            |_| {},
        )
        .await;
        assert_eq!(result, ForwardEventResult::Continue);
        assert_eq!(skipped_events, 1);

        let receive_task = tokio::spawn(async move {
            let mut events = Vec::new();
            for _ in 0..5 {
                events.push(
                    timeout(Duration::from_secs(2), event_rx.recv())
                        .await
                        .expect("event should arrive before timeout")
                        .expect("event stream should stay open"),
                );
            }
            events
        });

        for notification in [
            agent_message_delta_notification("hello"),
            item_completed_notification("hello"),
            turn_completed_notification(),
        ] {
            let result = forward_in_process_event(
                &event_tx,
                &mut skipped_events,
                InProcessServerEvent::ServerNotification(notification),
                |_| {},
            )
            .await;
            assert_eq!(result, ForwardEventResult::Continue);
        }
        assert_eq!(skipped_events, 0);

        let events = receive_task
            .await
            .expect("receiver task should join successfully");
        assert!(matches!(
            &events[0],
            InProcessServerEvent::ServerNotification(
                ServerNotification::CommandExecutionOutputDelta(notification)
            ) if notification.delta == "stdout-1"
        ));
        assert!(matches!(
            &events[1],
            InProcessServerEvent::Lagged { skipped: 1 }
        ));
        assert!(matches!(
            &events[2],
            InProcessServerEvent::ServerNotification(ServerNotification::AgentMessageDelta(
                notification
            )) if notification.delta == "hello"
        ));
        assert!(matches!(
            &events[3],
            InProcessServerEvent::ServerNotification(ServerNotification::ItemCompleted(
                notification
            )) if matches!(
                &notification.item,
                codex_app_server_protocol::ThreadItem::AgentMessage { text, .. } if text == "hello"
            )
        ));
        assert!(matches!(
            &events[4],
            InProcessServerEvent::ServerNotification(ServerNotification::TurnCompleted(
                notification
            )) if notification.turn.status == codex_app_server_protocol::TurnStatus::Completed
        ));
    }
    #[tokio::test]
    async fn remote_unix_socket_typed_request_roundtrip_works() {
        let socket_dir = TempDir::new().expect("socket dir");
        let socket_path = AbsolutePathBuf::from_absolute_path(socket_dir.path().join("codex.sock"))
            .expect("socket path should resolve");
        let mut listener = UnixListener::bind(socket_path.as_path())
            .await
            .expect("listener should bind");
        tokio::spawn(async move {
            let stream = listener.accept().await.expect("accept should succeed");
            let mut websocket = accept_async(stream)
                .await
                .expect("websocket upgrade should succeed");
            expect_remote_initialize(&mut websocket).await;
            let JSONRPCMessage::Request(request) = read_websocket_message(&mut websocket).await
            else {
                panic!("expected account/read request");
            };
            assert_eq!(request.method, "account/read");
            write_websocket_message(
                &mut websocket,
                JSONRPCMessage::Response(JSONRPCResponse {
                    id: request.id,
                    result: serde_json::to_value(GetAccountResponse {
                        account: None,
                        requires_openai_auth: false,
                    })
                    .expect("response should serialize"),
                }),
            )
            .await;
            websocket.close(None).await.expect("close should succeed");
        });
        let client = RemoteAppServerClient::connect(RemoteAppServerConnectArgs {
            endpoint: RemoteAppServerEndpoint::UnixSocket { socket_path },
            client_name: "codex-app-server-client-test".to_string(),
            client_version: "0.0.0-test".to_string(),
            opt_out_notification_methods: Vec::new(),
            channel_capacity: 8,
        })
        .await
        .expect("remote client should connect");

        let response: GetAccountResponse = client
            .request_typed(ClientRequest::GetAccount {
                request_id: RequestId::Integer(1),
                params: codex_app_server_protocol::GetAccountParams {
                    refresh_token: false,
                },
            })
            .await
            .expect("typed request should succeed");
        assert_eq!(response.account, None);

        client.shutdown().await.expect("shutdown should complete");
    }
    #[test]
    fn typed_request_error_exposes_sources() {
        let transport = TypedRequestError::Transport {
            method: "config/read".to_string(),
            source: IoError::new(ErrorKind::BrokenPipe, "closed"),
        };
        assert_eq!(std::error::Error::source(&transport).is_some(), true);

        let server = TypedRequestError::Server {
            method: "thread/read".to_string(),
            source: JSONRPCErrorError {
                code: -32603,
                data: Some(serde_json::json!({"detail": "config lock mismatch"})),
                message: "internal".to_string(),
            },
        };
        assert_eq!(std::error::Error::source(&server).is_some(), false);
        assert_eq!(
            server.to_string(),
            "thread/read failed: internal (code -32603), data: {\"detail\":\"config lock mismatch\"}"
        );

        let deserialize = TypedRequestError::Deserialize {
            method: "thread/start".to_string(),
            source: serde_json::from_str::<u32>("\"nope\"")
                .expect_err("invalid integer should return deserialize error"),
        };
        assert_eq!(std::error::Error::source(&deserialize).is_some(), true);
    }

    #[tokio::test]
    async fn next_event_surfaces_lagged_markers() {
        let (command_tx, _command_rx) = mpsc::channel(1);
        let (event_tx, event_rx) = mpsc::channel(1);
        let worker_handle = tokio::spawn(async {});
        event_tx
            .send(InProcessServerEvent::Lagged { skipped: 3 })
            .await
            .expect("lagged marker should enqueue");
        drop(event_tx);

        let mut client = InProcessAppServerClient {
            command_tx,
            event_rx,
            worker_handle,
        };

        let event = timeout(Duration::from_secs(2), client.next_event())
            .await
            .expect("lagged marker should arrive before timeout");
        assert!(matches!(
            event,
            Some(InProcessServerEvent::Lagged { skipped: 3 })
        ));

        client.shutdown().await.expect("shutdown should complete");
    }

    #[test]
    fn event_requires_delivery_marks_transcript_and_terminal_events() {
        assert!(event_requires_delivery(
            &InProcessServerEvent::ServerNotification(
                codex_app_server_protocol::ServerNotification::TurnCompleted(
                    codex_app_server_protocol::TurnCompletedNotification {
                        thread_id: "thread".to_string(),
                        turn: codex_app_server_protocol::Turn {
                            id: "turn".to_string(),
                            items_view: codex_app_server_protocol::TurnItemsView::Full,
                            items: Vec::new(),
                            status: codex_app_server_protocol::TurnStatus::Completed,
                            error: None,
                            started_at: None,
                            completed_at: Some(0),
                            duration_ms: None,
                        },
                    }
                )
            )
        ));
        assert!(event_requires_delivery(
            &InProcessServerEvent::ServerNotification(
                codex_app_server_protocol::ServerNotification::AgentMessageDelta(
                    codex_app_server_protocol::AgentMessageDeltaNotification {
                        thread_id: "thread".to_string(),
                        turn_id: "turn".to_string(),
                        item_id: "item".to_string(),
                        delta: "hello".to_string(),
                    }
                )
            )
        ));
        assert!(event_requires_delivery(
            &InProcessServerEvent::ServerNotification(
                codex_app_server_protocol::ServerNotification::ItemCompleted(
                    codex_app_server_protocol::ItemCompletedNotification {
                        thread_id: "thread".to_string(),
                        turn_id: "turn".to_string(),
                        completed_at_ms: 0,
                        item: codex_app_server_protocol::ThreadItem::AgentMessage {
                            id: "item".to_string(),
                            text: "hello".to_string(),
                            phase: None,
                        },
                    }
                )
            )
        ));
        assert!(!event_requires_delivery(&InProcessServerEvent::Lagged {
            skipped: 1
        }));
        assert!(!event_requires_delivery(
            &InProcessServerEvent::ServerNotification(
                codex_app_server_protocol::ServerNotification::CommandExecutionOutputDelta(
                    codex_app_server_protocol::CommandExecutionOutputDeltaNotification {
                        thread_id: "thread".to_string(),
                        turn_id: "turn".to_string(),
                        item_id: "item".to_string(),
                        delta: "stdout".to_string(),
                    }
                )
            )
        ));
    }

    #[tokio::test]
    async fn shutdown_completes_promptly_without_retained_managers() {
        let client = start_test_client(SessionSource::Cli).await;

        timeout(Duration::from_secs(1), client.shutdown())
            .await
            .expect("shutdown should not wait for the 5s fallback timeout")
            .expect("shutdown should complete");
    }
}
