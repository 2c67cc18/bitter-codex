use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use axum::http::HeaderValue;
use codex_login::default_client::SetOriginatorError;
use codex_login::default_client::USER_AGENT_SUFFIX;
use codex_login::default_client::get_codex_user_agent;
use codex_login::default_client::set_default_originator;

use super::*;
use crate::message_processor::ConnectionSessionState;
use crate::message_processor::InitializedConnectionSessionState;

const NON_ORIGINATING_CLIENT_NAMES: &[&str] = &["codex_app_server_daemon", "codex-backend"];

#[derive(Clone)]
pub(crate) struct InitializeRequestProcessor {
    outgoing: Arc<OutgoingMessageSender>,
    config: Arc<Config>,
    config_warnings: Arc<Vec<ConfigWarningNotification>>,
}

impl InitializeRequestProcessor {
    pub(crate) fn new(
        outgoing: Arc<OutgoingMessageSender>,
        config: Arc<Config>,
        config_warnings: Vec<ConfigWarningNotification>,
    ) -> Self {
        Self {
            outgoing,
            config,
            config_warnings: Arc::new(config_warnings),
        }
    }

    pub(crate) async fn initialize(
        &self,
        connection_id: ConnectionId,
        request_id: RequestId,
        params: InitializeParams,
        session: &ConnectionSessionState,

        outbound_initialized: Option<&AtomicBool>,
    ) -> Result<bool, JSONRPCErrorError> {
        let connection_request_id = ConnectionRequestId {
            connection_id,
            request_id,
        };
        if session.initialized() {
            return Err(invalid_request("Already initialized"));
        }

        let opt_out_notification_methods = params
            .capabilities
            .and_then(|capabilities| capabilities.opt_out_notification_methods)
            .unwrap_or_default();
        let ClientInfo {
            name,
            title: _title,
            version,
        } = params.client_info;

        if HeaderValue::from_str(&name).is_err() {
            return Err(invalid_request(format!(
                "Invalid clientInfo.name: '{name}'. Must be a valid HTTP header value."
            )));
        }
        let originator = name.clone();
        let user_agent_suffix = format!("{name}; {version}");
        let mutates_global_identity = !NON_ORIGINATING_CLIENT_NAMES.contains(&name.as_str());
        let codex_home = self.config.codex_home.clone();
        if session
            .initialize(InitializedConnectionSessionState {
                opted_out_notification_methods: opt_out_notification_methods.into_iter().collect(),
                app_server_client_name: name.clone(),
                client_version: version,
            })
            .is_err()
        {
            return Err(invalid_request("Already initialized"));
        }

        if mutates_global_identity {
            if let Err(error) = set_default_originator(originator.clone()) {
                match error {
                    SetOriginatorError::InvalidHeaderValue => {
                        tracing::warn!(
                            client_info_name = %name,
                            "validated clientInfo.name was rejected while setting originator"
                        );
                    }
                    SetOriginatorError::AlreadyInitialized => {}
                }
            }
        }
        if mutates_global_identity && let Ok(mut suffix) = USER_AGENT_SUFFIX.lock() {
            *suffix = Some(user_agent_suffix);
        }

        let user_agent = get_codex_user_agent();
        let response = InitializeResponse {
            user_agent,
            codex_home,
            platform_family: std::env::consts::FAMILY.to_string(),
            platform_os: std::env::consts::OS.to_string(),
        };

        self.outgoing
            .send_response(connection_request_id, response)
            .await;

        if let Some(outbound_initialized) = outbound_initialized {
            outbound_initialized.store(true, Ordering::Release);
            return Ok(true);
        }

        Ok(false)
    }

    pub(crate) async fn send_initialize_notifications_to_connection(
        &self,
        connection_id: ConnectionId,
    ) {
        for notification in self.config_warnings.iter().cloned() {
            self.outgoing
                .send_server_notification_to_connections(
                    &[connection_id],
                    ServerNotification::ConfigWarning(notification),
                )
                .await;
        }
    }

    pub(crate) async fn send_initialize_notifications(&self) {
        for notification in self.config_warnings.iter().cloned() {
            self.outgoing
                .send_server_notification(ServerNotification::ConfigWarning(notification))
                .await;
        }
    }
}
