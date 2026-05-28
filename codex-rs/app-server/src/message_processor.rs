use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;

use crate::config_manager::ConfigManager;
use crate::connection_rpc_gate::ConnectionRpcGate;
use crate::error_code::invalid_request;
use crate::outgoing_message::ConnectionId;
use crate::outgoing_message::ConnectionRequestId;
use crate::outgoing_message::OutgoingMessageSender;
use crate::outgoing_message::RequestContext;
use crate::request_processors::AccountRequestProcessor;
use crate::request_processors::ConfigRequestProcessor;
use crate::request_processors::InitializeRequestProcessor;
use crate::request_processors::ModelRequestProcessor;
use crate::request_processors::ThreadRequestProcessor;
use crate::request_processors::TurnRequestProcessor;
use crate::request_serialization::QueuedInitializedRequest;
use crate::request_serialization::RequestSerializationQueueKey;
use crate::request_serialization::RequestSerializationQueues;
use crate::thread_state::ConnectionCapabilities;
use crate::thread_state::ThreadStateManager;
use crate::transport::AppServerTransport;
use async_trait::async_trait;
use codex_app_server_protocol::AuthMode as LoginAuthMode;
use codex_app_server_protocol::ChatgptAuthTokensRefreshParams;
use codex_app_server_protocol::ChatgptAuthTokensRefreshReason;
use codex_app_server_protocol::ChatgptAuthTokensRefreshResponse;
use codex_app_server_protocol::ClientNotification;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::ClientResponsePayload;
use codex_app_server_protocol::ConfigWarningNotification;
use codex_app_server_protocol::JSONRPCError;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::JSONRPCNotification;
use codex_app_server_protocol::JSONRPCRequest;
use codex_app_server_protocol::JSONRPCResponse;
use codex_app_server_protocol::ServerRequestPayload;
use codex_arg0::Arg0DispatchPaths;
use codex_core::ThreadManager;
use codex_core::config::Config;
use codex_login::AuthManager;
use codex_login::auth::ExternalAuth;
use codex_login::auth::ExternalAuthRefreshContext;
use codex_login::auth::ExternalAuthRefreshReason;
use codex_login::auth::ExternalAuthTokens;
use codex_protocol::ThreadId;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::W3cTraceContext;
use codex_rollout::StateDbHandle;
use tokio::sync::Mutex;
use tokio::sync::Semaphore;
use tokio::sync::broadcast;
use tokio::sync::watch;
use tokio::time::Duration;
use tokio::time::timeout;
use tracing::Instrument;

const EXTERNAL_AUTH_REFRESH_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
struct ExternalAuthRefreshBridge {
    outgoing: Arc<OutgoingMessageSender>,
}

impl ExternalAuthRefreshBridge {
    fn map_reason(reason: ExternalAuthRefreshReason) -> ChatgptAuthTokensRefreshReason {
        match reason {
            ExternalAuthRefreshReason::Unauthorized => ChatgptAuthTokensRefreshReason::Unauthorized,
        }
    }
}

#[async_trait]
impl ExternalAuth for ExternalAuthRefreshBridge {
    fn auth_mode(&self) -> LoginAuthMode {
        LoginAuthMode::Chatgpt
    }

    async fn refresh(
        &self,
        context: ExternalAuthRefreshContext,
    ) -> std::io::Result<ExternalAuthTokens> {
        let params = ChatgptAuthTokensRefreshParams {
            reason: Self::map_reason(context.reason),
            previous_account_id: context.previous_account_id,
        };

        let (request_id, rx) = self
            .outgoing
            .send_request(ServerRequestPayload::ChatgptAuthTokensRefresh(params))
            .await;

        let result = match timeout(EXTERNAL_AUTH_REFRESH_TIMEOUT, rx).await {
            Ok(result) => {
                let result = result.map_err(|err| {
                    std::io::Error::other(format!("auth refresh request canceled: {err}"))
                })?;
                result.map_err(|err| {
                    std::io::Error::other(format!(
                        "auth refresh request failed: code={} message={}",
                        err.code, err.message
                    ))
                })?
            }
            Err(_) => {
                let _canceled = self.outgoing.cancel_request(&request_id).await;
                return Err(std::io::Error::other(format!(
                    "auth refresh request timed out after {}s",
                    EXTERNAL_AUTH_REFRESH_TIMEOUT.as_secs()
                )));
            }
        };

        let response: ChatgptAuthTokensRefreshResponse =
            serde_json::from_value(result).map_err(std::io::Error::other)?;

        Ok(ExternalAuthTokens::chatgpt(
            response.access_token,
            response.chatgpt_account_id,
            response.chatgpt_plan_type,
        ))
    }
}

pub(crate) struct MessageProcessor {
    outgoing: Arc<OutgoingMessageSender>,
    account_processor: AccountRequestProcessor,
    config_processor: ConfigRequestProcessor,
    initialize_processor: InitializeRequestProcessor,
    model_processor: ModelRequestProcessor,
    thread_processor: ThreadRequestProcessor,
    turn_processor: TurnRequestProcessor,
    request_serialization_queues: RequestSerializationQueues,
}

#[derive(Debug)]
pub(crate) struct ConnectionSessionState {
    pub(crate) rpc_gate: Arc<ConnectionRpcGate>,
    initialized: OnceLock<InitializedConnectionSessionState>,
}

#[derive(Debug)]
pub(crate) struct InitializedConnectionSessionState {
    pub(crate) opted_out_notification_methods: HashSet<String>,
    pub(crate) app_server_client_name: String,
    pub(crate) client_version: String,
}

impl Default for ConnectionSessionState {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnectionSessionState {
    pub(crate) fn new() -> Self {
        Self {
            rpc_gate: Arc::new(ConnectionRpcGate::new()),
            initialized: OnceLock::new(),
        }
    }

    pub(crate) fn initialized(&self) -> bool {
        self.initialized.get().is_some()
    }

    pub(crate) fn opted_out_notification_methods(&self) -> HashSet<String> {
        self.initialized
            .get()
            .map(|session| session.opted_out_notification_methods.clone())
            .unwrap_or_default()
    }

    pub(crate) fn app_server_client_name(&self) -> Option<&str> {
        self.initialized
            .get()
            .map(|session| session.app_server_client_name.as_str())
    }

    pub(crate) fn client_version(&self) -> Option<&str> {
        self.initialized
            .get()
            .map(|session| session.client_version.as_str())
    }

    pub(crate) fn initialize(&self, session: InitializedConnectionSessionState) -> Result<(), ()> {
        self.initialized.set(session).map_err(|_| ())
    }
}

pub(crate) struct MessageProcessorArgs {
    pub(crate) outgoing: Arc<OutgoingMessageSender>,
    pub(crate) arg0_paths: Arg0DispatchPaths,
    pub(crate) config: Arc<Config>,
    pub(crate) config_manager: ConfigManager,
    pub(crate) state_db: Option<StateDbHandle>,
    pub(crate) config_warnings: Vec<ConfigWarningNotification>,
    pub(crate) session_source: SessionSource,
    pub(crate) auth_manager: Arc<AuthManager>,
    pub(crate) installation_id: String,
}

impl MessageProcessor {
    pub(crate) fn new(args: MessageProcessorArgs) -> Self {
        let MessageProcessorArgs {
            outgoing,
            arg0_paths,
            config,
            config_manager,
            state_db,
            config_warnings,
            session_source,
            auth_manager,
            installation_id,
        } = args;
        auth_manager.set_external_auth(Arc::new(ExternalAuthRefreshBridge {
            outgoing: outgoing.clone(),
        }));
        let thread_state_manager = ThreadStateManager::new();

        let thread_store = codex_core::thread_store_from_config(config.as_ref(), state_db.clone());
        let thread_manager = Arc::new(ThreadManager::new(
            config.as_ref(),
            auth_manager.clone(),
            session_source,
            Arc::clone(&thread_store),
            state_db.clone(),
            installation_id,
        ));

        let pending_thread_unloads = Arc::new(Mutex::new(HashSet::new()));
        let thread_watch_manager =
            crate::thread_status::ThreadWatchManager::new_with_outgoing(outgoing.clone());
        let thread_list_state_permit = Arc::new(Semaphore::new(1));
        let account_processor = AccountRequestProcessor::new(
            auth_manager.clone(),
            Arc::clone(&thread_manager),
            outgoing.clone(),
            Arc::clone(&config),
        );
        let initialize_processor =
            InitializeRequestProcessor::new(outgoing.clone(), Arc::clone(&config), config_warnings);
        let model_processor = ModelRequestProcessor::new(Arc::clone(&thread_manager));
        let thread_processor = ThreadRequestProcessor::new(
            auth_manager.clone(),
            Arc::clone(&thread_manager),
            outgoing.clone(),
            arg0_paths.clone(),
            Arc::clone(&config),
            config_manager.clone(),
            Arc::clone(&thread_store),
            Arc::clone(&pending_thread_unloads),
            thread_state_manager.clone(),
            thread_watch_manager.clone(),
            Arc::clone(&thread_list_state_permit),
            state_db,
        );
        let turn_processor = TurnRequestProcessor::new(
            Arc::clone(&thread_manager),
            outgoing.clone(),
            thread_state_manager,
        );
        let config_processor = ConfigRequestProcessor::new(config_manager.clone());
        Self {
            outgoing,
            account_processor,
            config_processor,
            initialize_processor,
            model_processor,
            thread_processor,
            turn_processor,
            request_serialization_queues: RequestSerializationQueues::default(),
        }
    }

    pub(crate) fn clear_runtime_references(&self) {
        self.account_processor.clear_external_auth();
    }

    pub(crate) async fn process_request(
        self: &Arc<Self>,
        connection_id: ConnectionId,
        request: JSONRPCRequest,
        transport: &AppServerTransport,
        session: Arc<ConnectionSessionState>,
    ) {
        let request_method = request.method.as_str();
        tracing::trace!(
            ?connection_id,
            request_id = ?request.id,
            "app-server request: {request_method}"
        );
        let request_id = ConnectionRequestId {
            connection_id,
            request_id: request.id.clone(),
        };
        let request_span =
            crate::app_server_tracing::request_span(&request, transport, connection_id, &session);
        let request_trace = request.trace.as_ref().map(|trace| W3cTraceContext {
            traceparent: trace.traceparent.clone(),
            tracestate: trace.tracestate.clone(),
        });
        let request_context = RequestContext::new(request_id.clone(), request_span, request_trace);
        Self::run_request_with_context(
            Arc::clone(&self.outgoing),
            request_context.clone(),
            async {
                let codex_request = serde_json::to_value(&request)
                    .map_err(|err| invalid_request(format!("Invalid request: {err}")))
                    .and_then(|request_json| {
                        serde_json::from_value::<ClientRequest>(request_json)
                            .map_err(|err| invalid_request(format!("Invalid request: {err}")))
                    });
                let result = match codex_request {
                    Ok(codex_request) => {
                        self.handle_client_request(
                            request_id.clone(),
                            codex_request,
                            Arc::clone(&session),
                            None,
                            request_context.clone(),
                        )
                        .await
                    }
                    Err(error) => Err(error),
                };
                if let Err(error) = result {
                    self.outgoing.send_error(request_id.clone(), error).await;
                }
            },
        )
        .await;
    }

    pub(crate) async fn process_client_request(
        self: &Arc<Self>,
        connection_id: ConnectionId,
        request: ClientRequest,
        session: Arc<ConnectionSessionState>,
        outbound_initialized: &AtomicBool,
    ) {
        let request_id = ConnectionRequestId {
            connection_id,
            request_id: request.id().clone(),
        };
        let request_span =
            crate::app_server_tracing::typed_request_span(&request, connection_id, &session);
        let request_context = RequestContext::new(request_id.clone(), request_span, None);
        tracing::trace!(
            ?connection_id,
            request_id = ?request_id.request_id,
            "app-server typed request"
        );
        Self::run_request_with_context(
            Arc::clone(&self.outgoing),
            request_context.clone(),
            async {
                let result = self
                    .handle_client_request(
                        request_id.clone(),
                        request,
                        Arc::clone(&session),
                        Some(outbound_initialized),
                        request_context.clone(),
                    )
                    .await;
                if let Err(error) = result {
                    self.outgoing.send_error(request_id.clone(), error).await;
                }
            },
        )
        .await;
    }

    pub(crate) async fn process_notification(&self, notification: JSONRPCNotification) {
        tracing::info!("<- notification: {:?}", notification);
    }

    pub(crate) async fn process_client_notification(&self, notification: ClientNotification) {
        tracing::info!("<- typed notification: {:?}", notification);
    }

    async fn run_request_with_context<F>(
        outgoing: Arc<OutgoingMessageSender>,
        request_context: RequestContext,
        request_fut: F,
    ) where
        F: Future<Output = ()>,
    {
        outgoing
            .register_request_context(request_context.clone())
            .await;
        request_fut.instrument(request_context.span()).await;
    }

    pub(crate) fn thread_created_receiver(&self) -> broadcast::Receiver<ThreadId> {
        self.thread_processor.thread_created_receiver()
    }

    pub(crate) async fn send_initialize_notifications_to_connection(
        &self,
        connection_id: ConnectionId,
    ) {
        self.initialize_processor
            .send_initialize_notifications_to_connection(connection_id)
            .await;
    }

    pub(crate) async fn connection_initialized(&self, connection_id: ConnectionId) {
        self.thread_processor
            .connection_initialized(connection_id, ConnectionCapabilities)
            .await;
    }

    pub(crate) async fn send_initialize_notifications(&self) {
        self.initialize_processor
            .send_initialize_notifications()
            .await;
    }

    pub(crate) async fn try_attach_thread_listener(
        &self,
        thread_id: ThreadId,
        connection_ids: Vec<ConnectionId>,
    ) {
        self.thread_processor
            .try_attach_thread_listener(thread_id, connection_ids)
            .await;
    }

    pub(crate) async fn drain_background_tasks(&self) {
        self.thread_processor.drain_background_tasks().await;
    }

    pub(crate) async fn cancel_active_login(&self) {
        self.account_processor.cancel_active_login().await;
    }

    pub(crate) async fn clear_all_thread_listeners(&self) {
        self.thread_processor.clear_all_thread_listeners().await;
    }

    pub(crate) async fn shutdown_threads(&self) {
        self.thread_processor.shutdown_threads().await;
    }

    pub(crate) async fn connection_closed(
        &self,
        connection_id: ConnectionId,
        session_state: &ConnectionSessionState,
    ) {
        session_state.rpc_gate.shutdown().await;
        self.outgoing.connection_closed(connection_id).await;
        self.thread_processor.connection_closed(connection_id).await;
    }

    pub(crate) fn subscribe_running_assistant_turn_count(&self) -> watch::Receiver<usize> {
        self.thread_processor
            .subscribe_running_assistant_turn_count()
    }

    pub(crate) async fn process_response(&self, response: JSONRPCResponse) {
        tracing::info!("<- response: {:?}", response);
        let JSONRPCResponse { id, result, .. } = response;
        self.outgoing.notify_client_response(id, result).await
    }

    pub(crate) async fn process_error(&self, err: JSONRPCError) {
        tracing::error!("<- error: {:?}", err);
        self.outgoing.notify_client_error(err.id, err.error).await;
    }

    async fn handle_client_request(
        self: &Arc<Self>,
        connection_request_id: ConnectionRequestId,
        codex_request: ClientRequest,
        session: Arc<ConnectionSessionState>,

        outbound_initialized: Option<&AtomicBool>,
        request_context: RequestContext,
    ) -> Result<(), JSONRPCErrorError> {
        let connection_id = connection_request_id.connection_id;
        if let ClientRequest::Initialize { request_id, params } = codex_request {
            let connection_initialized = self
                .initialize_processor
                .initialize(
                    connection_id,
                    request_id,
                    params,
                    &session,
                    outbound_initialized,
                )
                .await?;
            if connection_initialized {
                self.thread_processor
                    .connection_initialized(connection_id, ConnectionCapabilities)
                    .await;
            }
            return Ok(());
        }

        self.dispatch_initialized_client_request(
            connection_request_id,
            codex_request,
            session,
            request_context,
        )
        .await
    }

    async fn dispatch_initialized_client_request(
        self: &Arc<Self>,
        connection_request_id: ConnectionRequestId,
        codex_request: ClientRequest,
        session: Arc<ConnectionSessionState>,
        request_context: RequestContext,
    ) -> Result<(), JSONRPCErrorError> {
        if !session.initialized() {
            return Err(invalid_request("Not initialized"));
        }

        let serialization_scope = codex_request.serialization_scope();
        let app_server_client_name = session.app_server_client_name().map(str::to_string);
        let client_version = session.client_version().map(str::to_string);
        let error_request_id = connection_request_id.clone();
        let rpc_gate = Arc::clone(&session.rpc_gate);
        let processor = Arc::clone(self);
        let span = request_context.span();
        let request = QueuedInitializedRequest::new(
            rpc_gate,
            async move {
                let processor_for_request = Arc::clone(&processor);
                let result = processor_for_request
                    .handle_initialized_client_request(
                        connection_request_id,
                        codex_request,
                        request_context,
                        app_server_client_name,
                        client_version,
                    )
                    .await;
                if let Err(error) = result {
                    processor.outgoing.send_error(error_request_id, error).await;
                }
            }
            .instrument(span),
        );

        if let Some(scope) = serialization_scope {
            let (key, access) = RequestSerializationQueueKey::from_scope(scope);
            self.request_serialization_queues
                .enqueue(key, access, request)
                .await;
        } else {
            tokio::spawn(async move {
                request.run().await;
            });
        }
        Ok(())
    }

    async fn handle_initialized_client_request(
        self: Arc<Self>,
        connection_request_id: ConnectionRequestId,
        codex_request: ClientRequest,
        request_context: RequestContext,
        app_server_client_name: Option<String>,
        client_version: Option<String>,
    ) -> Result<(), JSONRPCErrorError> {
        let connection_id = connection_request_id.connection_id;
        let request_id = ConnectionRequestId {
            connection_id,
            request_id: codex_request.id().clone(),
        };

        let result: Result<Option<ClientResponsePayload>, JSONRPCErrorError> = match codex_request {
            ClientRequest::Initialize { .. } => {
                panic!("Initialize should be handled before initialized request dispatch");
            }
            ClientRequest::ConfigRead { params, .. } => self
                .config_processor
                .read(params)
                .await
                .map(|response| Some(response.into())),
            ClientRequest::ConfigValueWrite { params, .. } => {
                self.config_processor.value_write(params).await.map(Some)
            }
            ClientRequest::ConfigBatchWrite { params, .. } => {
                self.config_processor.batch_write(params).await.map(Some)
            }
            ClientRequest::ModelProviderCapabilitiesRead { params: _, .. } => self
                .config_processor
                .model_provider_capabilities_read()
                .await
                .map(|response| Some(response.into())),
            ClientRequest::ThreadStart { params, .. } => {
                self.thread_processor
                    .thread_start(
                        request_id.clone(),
                        params,
                        app_server_client_name.clone(),
                        client_version.clone(),
                        request_context,
                    )
                    .await
            }
            ClientRequest::ThreadUnsubscribe { params, .. } => {
                self.thread_processor
                    .thread_unsubscribe(&request_id, params)
                    .await
            }
            ClientRequest::ThreadResume { params, .. } => {
                self.thread_processor
                    .thread_resume(
                        request_id.clone(),
                        params,
                        app_server_client_name.clone(),
                        client_version.clone(),
                    )
                    .await
            }
            ClientRequest::ThreadFork { params, .. } => {
                self.thread_processor
                    .thread_fork(
                        request_id.clone(),
                        params,
                        app_server_client_name.clone(),
                        client_version.clone(),
                    )
                    .await
            }
            ClientRequest::ThreadArchive { params, .. } => {
                self.thread_processor
                    .thread_archive(request_id.clone(), params)
                    .await
            }
            ClientRequest::ThreadSetName { params, .. } => {
                self.thread_processor
                    .thread_set_name(request_id.clone(), params)
                    .await
            }
            ClientRequest::ThreadMetadataUpdate { params, .. } => {
                self.thread_processor.thread_metadata_update(params).await
            }
            ClientRequest::ThreadSettingsUpdate { params, .. } => {
                self.turn_processor
                    .thread_settings_update(&request_id, params)
                    .await
            }
            ClientRequest::ThreadUnarchive { params, .. } => {
                self.thread_processor
                    .thread_unarchive(request_id.clone(), params)
                    .await
            }
            ClientRequest::ThreadCompactStart { params, .. } => {
                self.thread_processor
                    .thread_compact_start(&request_id, params)
                    .await
            }
            ClientRequest::ThreadBackgroundTerminalsClean { params, .. } => {
                self.thread_processor
                    .thread_background_terminals_clean(&request_id, params)
                    .await
            }
            ClientRequest::ThreadList { params, .. } => {
                self.thread_processor.thread_list(params).await
            }
            ClientRequest::ThreadLoadedList { params, .. } => {
                self.thread_processor.thread_loaded_list(params).await
            }
            ClientRequest::ThreadRead { params, .. } => {
                self.thread_processor.thread_read(params).await
            }
            ClientRequest::ThreadTurnsList { params, .. } => {
                self.thread_processor.thread_turns_list(params).await
            }
            ClientRequest::ModelList { params, .. } => {
                self.model_processor.model_list(params).await
            }
            ClientRequest::TurnStart { params, .. } => {
                self.turn_processor
                    .turn_start(
                        request_id.clone(),
                        params,
                        app_server_client_name.clone(),
                        client_version.clone(),
                    )
                    .await
            }
            ClientRequest::TurnSteer { params, .. } => {
                self.turn_processor.turn_steer(&request_id, params).await
            }
            ClientRequest::TurnInterrupt { params, .. } => {
                self.turn_processor
                    .turn_interrupt(&request_id, params)
                    .await
            }
            ClientRequest::LoginAccount { params, .. } => {
                self.account_processor
                    .login_account(request_id.clone(), params)
                    .await
            }
            ClientRequest::LogoutAccount { .. } => {
                self.account_processor
                    .logout_account(request_id.clone())
                    .await
            }
            ClientRequest::CancelLoginAccount { params, .. } => {
                self.account_processor.cancel_login_account(params).await
            }
            ClientRequest::GetAccount { params, .. } => {
                self.account_processor.get_account(params).await
            }
            ClientRequest::GetAuthStatus { params, .. } => {
                self.account_processor.get_auth_status(params).await
            }
            ClientRequest::GetAccountRateLimits { .. } => {
                self.account_processor.get_account_rate_limits().await
            }
            ClientRequest::SendAddCreditsNudgeEmail { params, .. } => {
                self.account_processor
                    .send_add_credits_nudge_email(params)
                    .await
            }
        };

        match result {
            Ok(Some(response)) => {
                self.outgoing
                    .send_response_as(request_id.clone(), response)
                    .await;
            }
            Ok(None) => {}
            Err(error) => {
                self.outgoing.send_error(request_id.clone(), error).await;
            }
        }
        Ok(())
    }
}
