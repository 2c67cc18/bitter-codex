use crate::config::ConstraintResult;
use crate::session::Codex;
use crate::session::SessionSettingsUpdate;
use crate::session::SteerInputError;
use crate::session::Submission;
use codex_features::Feature;
use codex_otel::SessionTelemetry;
use codex_protocol::config_types::ReasoningSummary;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result as CodexResult;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::SessionConfiguredEvent;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::TokenUsageInfo;
use codex_protocol::protocol::TurnEnvironmentSelection;
use codex_protocol::protocol::W3cTraceContext;
use codex_protocol::user_input::UserInput;
use codex_thread_store::StoredThread;
use codex_thread_store::StoredThreadHistory;
use codex_thread_store::ThreadMetadataPatch;
use codex_thread_store::ThreadStoreError;
use codex_thread_store::ThreadStoreResult;
use codex_utils_absolute_path::AbsolutePathBuf;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use codex_rollout::state_db::StateDbHandle;

#[derive(Clone, Debug)]
pub struct ThreadConfigSnapshot {
    pub model: String,
    pub model_provider_id: String,
    pub service_tier: Option<String>,
    pub cwd: AbsolutePathBuf,
    pub workspace_roots: Vec<AbsolutePathBuf>,
    pub ephemeral: bool,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub reasoning_summary: Option<ReasoningSummary>,
    pub session_source: SessionSource,
}

#[derive(Clone, Default)]
pub struct CodexThreadSettingsOverrides {
    pub cwd: Option<PathBuf>,
    pub workspace_roots: Option<Vec<AbsolutePathBuf>>,
    pub model: Option<String>,
    pub effort: Option<Option<ReasoningEffort>>,
    pub summary: Option<ReasoningSummary>,
    pub service_tier: Option<Option<String>>,
}

pub struct CodexThread {
    pub(crate) codex: Codex,
    pub(crate) session_source: SessionSource,
    session_configured: SessionConfiguredEvent,
    rollout_path: Option<PathBuf>,
}

impl CodexThread {
    pub(crate) fn new(
        codex: Codex,
        session_configured: SessionConfiguredEvent,
        rollout_path: Option<PathBuf>,
        session_source: SessionSource,
    ) -> Self {
        Self {
            codex,
            session_source,
            session_configured,
            rollout_path,
        }
    }

    pub async fn submit(&self, op: Op) -> CodexResult<String> {
        self.codex.submit(op).await
    }

    pub fn session_telemetry(&self) -> SessionTelemetry {
        self.codex.session.services.session_telemetry.clone()
    }

    pub async fn shutdown_and_wait(&self) -> CodexResult<()> {
        self.codex.shutdown_and_wait().await
    }

    pub async fn wait_until_terminated(&self) {
        self.codex.session_loop_termination.clone().await;
    }

    #[doc(hidden)]
    pub async fn ensure_rollout_materialized(&self) {
        self.codex.session.ensure_rollout_materialized().await;
    }

    #[doc(hidden)]
    pub async fn flush_rollout(&self) -> std::io::Result<()> {
        self.codex.session.flush_rollout().await
    }

    pub async fn submit_with_trace(
        &self,
        op: Op,
        trace: Option<W3cTraceContext>,
    ) -> CodexResult<String> {
        self.codex.submit_with_trace(op, trace).await
    }

    pub async fn steer_input(
        &self,
        input: Vec<UserInput>,
        expected_turn_id: Option<&str>,
        responsesapi_client_metadata: Option<HashMap<String, String>>,
    ) -> Result<String, SteerInputError> {
        self.codex
            .steer_input(input, expected_turn_id, responsesapi_client_metadata)
            .await
    }

    pub async fn set_app_server_client_info(
        &self,
        app_server_client_name: Option<String>,
        app_server_client_version: Option<String>,
    ) -> ConstraintResult<()> {
        self.codex
            .set_app_server_client_info(app_server_client_name, app_server_client_version)
            .await
    }

    pub async fn preview_thread_settings_overrides(
        &self,
        overrides: CodexThreadSettingsOverrides,
    ) -> ConstraintResult<ThreadConfigSnapshot> {
        let updates = self.thread_settings_update(overrides).await;
        self.codex.session.preview_settings(&updates).await
    }

    async fn thread_settings_update(
        &self,
        overrides: CodexThreadSettingsOverrides,
    ) -> SessionSettingsUpdate {
        let CodexThreadSettingsOverrides {
            cwd,
            workspace_roots,
            model: _,
            effort: _,
            summary,
            service_tier,
        } = overrides;
        SessionSettingsUpdate {
            cwd,
            workspace_roots,
            reasoning_summary: summary,
            service_tier,
            ..Default::default()
        }
    }

    pub async fn submit_with_id(&self, sub: Submission) -> CodexResult<()> {
        self.codex.submit_with_id(sub).await
    }

    pub async fn next_event(&self) -> CodexResult<Event> {
        self.codex.next_event().await
    }

    pub async fn token_usage_info(&self) -> Option<TokenUsageInfo> {
        self.codex.session.token_usage_info().await
    }

    pub(crate) async fn inject_user_message_without_turn(&self, message: String) {
        let message = ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText { text: message }],
            phase: None,
        };
        let pending_item = match pending_message_input_item(&message) {
            Ok(pending_item) => pending_item,
            Err(err) => {
                debug_assert!(false, "session-prefix message append should succeed: {err}");
                return;
            }
        };
        if self
            .codex
            .session
            .inject_response_items(vec![pending_item])
            .await
            .is_err()
        {
            let turn_context = self.codex.session.new_default_turn().await;
            self.codex
                .session
                .record_conversation_items(turn_context.as_ref(), &[message])
                .await;
        }
    }

    #[cfg(test)]
    pub(crate) async fn append_message(&self, message: ResponseItem) -> CodexResult<String> {
        let submission_id = uuid::Uuid::new_v4().to_string();
        let pending_item = pending_message_input_item(&message)?;
        if let Err(items) = self
            .codex
            .session
            .inject_response_items(vec![pending_item])
            .await
        {
            self.codex
                .session
                .input_queue
                .queue_response_items_for_next_turn(items)
                .await;
            self.codex.session.maybe_start_turn_for_pending_work().await;
        }

        Ok(submission_id)
    }

    pub async fn inject_response_items(&self, items: Vec<ResponseItem>) -> CodexResult<()> {
        if items.is_empty() {
            return Err(CodexErr::InvalidRequest(
                "items must not be empty".to_string(),
            ));
        }

        let turn_context = self.codex.session.new_default_turn().await;
        if self.codex.session.reference_context_item().await.is_none() {
            self.codex
                .session
                .record_context_updates_and_set_reference_context_item(turn_context.as_ref())
                .await;
        }
        self.codex
            .session
            .record_conversation_items(turn_context.as_ref(), &items)
            .await;
        self.codex.session.flush_rollout().await?;
        Ok(())
    }

    pub fn rollout_path(&self) -> Option<PathBuf> {
        self.rollout_path.clone()
    }

    pub fn session_configured(&self) -> SessionConfiguredEvent {
        self.session_configured.clone()
    }

    pub(crate) fn is_running(&self) -> bool {
        !self.codex.tx_sub.is_closed()
    }

    pub async fn load_history(
        &self,
        include_archived: bool,
    ) -> ThreadStoreResult<StoredThreadHistory> {
        let live_thread = self
            .codex
            .session
            .live_thread_for_persistence("load history")
            .map_err(|err| ThreadStoreError::Internal {
                message: err.to_string(),
            })?;
        live_thread.load_history(include_archived).await
    }

    pub async fn read_thread(
        &self,
        include_archived: bool,
        include_history: bool,
    ) -> ThreadStoreResult<StoredThread> {
        let live_thread = self
            .codex
            .session
            .live_thread_for_persistence("read thread")
            .map_err(|err| ThreadStoreError::Internal {
                message: err.to_string(),
            })?;
        live_thread
            .read_thread(include_archived, include_history)
            .await
    }

    pub async fn update_thread_metadata(
        &self,
        patch: ThreadMetadataPatch,
        include_archived: bool,
    ) -> ThreadStoreResult<StoredThread> {
        let live_thread = self
            .codex
            .session
            .live_thread_for_persistence("update thread metadata")
            .map_err(|err| ThreadStoreError::Internal {
                message: err.to_string(),
            })?;
        live_thread.update_metadata(patch, include_archived).await
    }

    pub fn state_db(&self) -> Option<StateDbHandle> {
        self.codex.state_db()
    }

    pub async fn config_snapshot(&self) -> ThreadConfigSnapshot {
        self.codex.thread_config_snapshot().await
    }

    pub async fn config(&self) -> Arc<crate::config::Config> {
        self.codex.session.get_config().await
    }

    pub async fn refresh_runtime_config(&self, next_config: crate::config::Config) {
        self.codex.session.refresh_runtime_config(next_config).await;
    }

    pub async fn environment_selections(&self) -> Vec<TurnEnvironmentSelection> {
        self.codex.thread_environment_selections().await
    }

    pub fn enabled(&self, feature: Feature) -> bool {
        self.codex.enabled(feature)
    }
}

fn pending_message_input_item(message: &ResponseItem) -> CodexResult<ResponseInputItem> {
    match message {
        ResponseItem::Message {
            role,
            content,
            phase,
            ..
        } => Ok(ResponseInputItem::Message {
            role: role.clone(),
            content: content.clone(),
            phase: phase.clone(),
        }),
        _ => Err(CodexErr::InvalidRequest(
            "append_message only supports ResponseItem::Message".to_string(),
        )),
    }
}
