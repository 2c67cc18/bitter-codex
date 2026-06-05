use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use crate::config::ManagedFeatures;
use crate::context::ContextualUserFragment;
use crate::path_utils::normalize_for_native_workdir;
use crate::turn_metadata::TurnMetadataState;
use crate::turn_timing::now_unix_timestamp_ms;
use async_channel::Receiver;
use async_channel::Sender;
use chrono::Local;
use chrono::Utc;
use codex_features::Feature;
use codex_login::AuthManager;
use codex_login::CodexAuth;
use codex_login::auth_env_telemetry::collect_auth_env_telemetry;
use codex_login::default_client::originator;
use codex_models_manager::manager::SharedModelsManager;
use codex_otel::current_span_trace_id;
use codex_otel::current_span_w3c_trace_context;
use codex_otel::set_parent_from_w3c_trace_context;
use codex_protocol::SessionId;
use codex_protocol::ThreadId;
use codex_protocol::config_types::AutoCompactTokenLimitScope;
use codex_protocol::config_types::SERVICE_TIER_DEFAULT_REQUEST_VALUE;
use codex_protocol::items::TurnItem;
use codex_protocol::models::BaseInstructions;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::protocol::AdditionalContextEntry;
use codex_protocol::protocol::ItemCompletedEvent;
use codex_protocol::protocol::ItemStartedEvent;
use codex_protocol::protocol::RawResponseItemEvent;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::TurnAbortReason;
use codex_protocol::protocol::TurnContextItem;
use codex_protocol::protocol::TurnEnvironmentSelection;
use codex_protocol::protocol::W3cTraceContext;
use codex_protocol::protocol::WebToolRuntime;
use codex_rollout::state_db;
use codex_thread_store::CreateThreadParams;
use codex_thread_store::LiveThread;
use codex_thread_store::LiveThreadInitGuard;
use codex_thread_store::LocalThreadStore;
use codex_thread_store::ReadThreadParams;
use codex_thread_store::ResumeThreadParams;
use codex_thread_store::ThreadEventPersistenceMode;
use codex_thread_store::ThreadPersistenceMetadata;
use codex_thread_store::ThreadStore;
use codex_utils_output_truncation::TruncationPolicy;
use futures::future::BoxFuture;
use futures::future::Shared;
use futures::prelude::*;
use serde_json::Value;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::Instrument;
use tracing::debug;
use tracing::error;
use tracing::info;
use tracing::info_span;
use tracing::instrument;
use tracing::warn;
use uuid::Uuid;

use crate::client::ModelClient;
use crate::codex_thread::ThreadConfigSnapshot;
use crate::config::Config;
use crate::config::ConstraintResult;
use crate::context_manager::ContextManager;
use crate::context_manager::TotalTokenUsageBreakdown;
use crate::thread_rollout_truncation::initial_history_has_prior_user_turns;
use codex_config::CONFIG_TOML_FILE;
use codex_config::ConfigLayerSource;
use codex_config::ConfigLayerStackOrdering;
use codex_model_provider_info::ModelProviderInfo;
use codex_protocol::config_types::ShellEnvironmentPolicy;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result as CodexResult;

mod config_lock;
mod handlers;
mod input_queue;
mod rollout_reconstruction;
#[allow(clippy::module_inception)]
pub(crate) mod session;
pub(crate) mod turn;
pub(crate) mod turn_context;
use self::config_lock::export_config_lock_if_configured;
use self::config_lock::validate_config_lock_if_configured;
use self::handlers::submission_loop;
pub(crate) use self::input_queue::TurnInput;
pub(crate) use self::input_queue::TurnInputQueue;
use self::session::Session;
use self::session::SessionConfiguration;
pub(crate) use self::session::SessionSettingsUpdate;
use self::turn_context::ResolvedTurnEnvironments;
use self::turn_context::TurnContext;

#[derive(Debug, PartialEq)]
pub enum SteerInputError {
    NoActiveTurn(Vec<UserInput>),
    ExpectedTurnMismatch { expected: String, actual: String },
    ActiveTurnNotSteerable { turn_kind: NonSteerableTurnKind },
    EmptyInput,
}

impl SteerInputError {
    fn to_error_event(&self) -> ErrorEvent {
        match self {
            Self::NoActiveTurn(_) => ErrorEvent {
                message: "no active turn to steer".to_string(),
                codex_error_info: Some(CodexErrorInfo::BadRequest),
            },
            Self::ExpectedTurnMismatch { expected, actual } => ErrorEvent {
                message: format!("expected active turn id `{expected}` but found `{actual}`"),
                codex_error_info: Some(CodexErrorInfo::BadRequest),
            },
            Self::ActiveTurnNotSteerable { turn_kind } => {
                let turn_kind_label = match turn_kind {
                    NonSteerableTurnKind::Compact => "compact",
                };
                ErrorEvent {
                    message: format!("cannot steer a {turn_kind_label} turn"),
                    codex_error_info: Some(CodexErrorInfo::ActiveTurnNotSteerable {
                        turn_kind: *turn_kind,
                    }),
                }
            }
            Self::EmptyInput => ErrorEvent {
                message: "input must not be empty".to_string(),
                codex_error_info: Some(CodexErrorInfo::BadRequest),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreviousTurnSettings {
    pub(crate) model: String,
}

#[derive(Debug, Clone)]
pub struct Submission {
    pub id: String,
    pub op: Op,
    pub trace: Option<W3cTraceContext>,
}

use crate::rollout::map_session_init_error;
use crate::session_startup_prewarm::SessionStartupPrewarmHandle;
use crate::shell;
use crate::shell_snapshot::ShellSnapshot;
use crate::state::AutoCompactWindowSnapshot;
use crate::state::SessionServices;
use crate::state::SessionState;
use crate::state::TaskKind;
use crate::turn_timing::TurnTimingState;
use crate::turn_timing::record_turn_ttfm_metric;
use crate::unified_exec::UnifiedExecProcessManager;
use codex_git_utils::get_git_repo_root;
use codex_otel::SessionTelemetry;
use codex_otel::THREAD_STARTED_METRIC;
use codex_otel::TelemetryAuthMode;
use codex_protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use codex_protocol::dynamic_tools::DynamicToolResponse;
use codex_protocol::dynamic_tools::DynamicToolSpec;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use codex_protocol::protocol::CodexErrorInfo;
use codex_protocol::protocol::CompactedItem;
use codex_protocol::protocol::ErrorEvent;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::InitialHistory;
use codex_protocol::protocol::ModelVerification;
use codex_protocol::protocol::ModelVerificationEvent;
use codex_protocol::protocol::NonSteerableTurnKind;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::RateLimitSnapshot;
use codex_protocol::protocol::SessionConfiguredEvent;
use codex_protocol::protocol::StreamErrorEvent;
use codex_protocol::protocol::TokenCountEvent;
use codex_protocol::protocol::TokenUsage;
use codex_protocol::protocol::TokenUsageInfo;
use codex_protocol::protocol::WarningEvent;
use codex_protocol::user_input::UserInput;
use codex_utils_absolute_path::AbsolutePathBuf;

pub struct Codex {
    pub(crate) tx_sub: Sender<Submission>,
    pub(crate) rx_event: Receiver<Event>,

    pub(crate) session: Arc<Session>,

    pub(crate) session_loop_termination: SessionLoopTermination,
}

pub(crate) type SessionLoopTermination = Shared<BoxFuture<'static, ()>>;

pub struct CodexSpawnOk {
    pub codex: Codex,
    pub thread_id: ThreadId,
}

pub(crate) struct CodexSpawnArgs {
    pub(crate) config: Config,
    pub(crate) installation_id: String,
    pub(crate) auth_manager: Arc<AuthManager>,
    pub(crate) models_manager: SharedModelsManager,
    pub(crate) conversation_history: InitialHistory,
    pub(crate) session_source: SessionSource,
    pub(crate) forked_from_thread_id: Option<ThreadId>,
    pub(crate) dynamic_tools: Vec<DynamicToolSpec>,
    pub(crate) parent_session_id: Option<SessionId>,
    pub(crate) persist_extended_history: bool,
    pub(crate) metrics_service_name: Option<String>,
    pub(crate) inherited_shell_snapshot: Option<Arc<ShellSnapshot>>,
    pub(crate) user_shell_override: Option<shell::Shell>,
    pub(crate) parent_trace: Option<W3cTraceContext>,
    pub(crate) environment_selections: ResolvedTurnEnvironments,
    pub(crate) thread_store: Arc<dyn ThreadStore>,
}

pub(crate) const INITIAL_SUBMIT_ID: &str = "";
pub(crate) const SUBMISSION_CHANNEL_CAPACITY: usize = 512;

impl Codex {
    pub(crate) async fn spawn(args: CodexSpawnArgs) -> CodexResult<CodexSpawnOk> {
        let parent_trace = match args.parent_trace {
            Some(trace) => {
                if codex_otel::context_from_w3c_trace_context(&trace).is_some() {
                    Some(trace)
                } else {
                    warn!("ignoring invalid thread spawn trace carrier");
                    None
                }
            }
            None => None,
        };
        let thread_spawn_span = info_span!("thread_spawn", otel.name = "thread_spawn");
        if let Some(trace) = parent_trace.as_ref() {
            let _ = set_parent_from_w3c_trace_context(&thread_spawn_span, trace);
        }
        Self::spawn_internal(CodexSpawnArgs {
            parent_trace,
            ..args
        })
        .instrument(thread_spawn_span)
        .await
    }

    async fn spawn_internal(args: CodexSpawnArgs) -> CodexResult<CodexSpawnOk> {
        let CodexSpawnArgs {
            config,
            installation_id,
            auth_manager,
            models_manager,
            conversation_history,
            session_source,
            forked_from_thread_id,
            dynamic_tools,
            parent_session_id,
            persist_extended_history,
            metrics_service_name,
            inherited_shell_snapshot,
            user_shell_override,
            parent_trace: _,
            environment_selections,
            thread_store,
        } = args;
        let (tx_sub, rx_sub) = async_channel::bounded(SUBMISSION_CHANNEL_CAPACITY);
        let (tx_event, rx_event) = async_channel::unbounded();

        let config = Arc::new(config);
        let refresh_strategy = codex_models_manager::manager::RefreshStrategy::OnlineIfUncached;
        if config.model.is_none()
            || !matches!(
                refresh_strategy,
                codex_models_manager::manager::RefreshStrategy::Offline
            )
        {
            let _ = models_manager.list_models(refresh_strategy).await;
        }
        let model = models_manager
            .get_default_model(&config.model, refresh_strategy)
            .await;

        let model_info = models_manager
            .get_model_info(model.as_str(), &config.to_models_manager_config())
            .await;
        let base_instructions = config
            .base_instructions
            .clone()
            .or_else(|| conversation_history.get_base_instructions().map(|s| s.text))
            .unwrap_or_else(|| model_info.get_model_instructions());
        let dynamic_tools = if dynamic_tools.is_empty() {
            conversation_history.get_dynamic_tools().unwrap_or_default()
        } else {
            dynamic_tools
        };
        let service_tier = get_service_tier(config.service_tier.clone(), &model_info);
        let session_configuration = SessionConfiguration {
            provider: config.model_provider.clone(),
            model,
            model_reasoning_effort: config.model_reasoning_effort,
            model_reasoning_summary: config.model_reasoning_summary,
            service_tier,
            developer_instructions: config.developer_instructions.clone(),
            base_instructions,
            compact_prompt: config.compact_prompt.clone(),
            cwd: config.cwd.clone(),
            workspace_roots: config.workspace_roots.clone(),
            codex_home: config.codex_home.clone(),
            thread_name: None,
            environments: environment_selections.to_selections(),
            original_config_do_not_use: Arc::clone(&config),
            metrics_service_name,
            app_server_client_name: None,
            app_server_client_version: None,
            session_source,
            forked_from_thread_id,
            dynamic_tools,
            web_tool_runtime: WebToolRuntime::Hosted,
            persist_extended_history,
            inherited_shell_snapshot,
            user_shell_override,
        };

        let session_source_clone = session_configuration.session_source.clone();
        let session = Session::new(
            session_configuration,
            config.clone(),
            installation_id,
            auth_manager.clone(),
            models_manager.clone(),
            tx_event.clone(),
            conversation_history,
            session_source_clone,
            parent_session_id,
            thread_store,
        )
        .await
        .map_err(|e| {
            error!("Failed to create session: {e:#}");
            map_session_init_error(&e, &config.codex_home)
        })?;
        let thread_id = session.conversation_id;

        let session_for_loop = Arc::clone(&session);
        let session_loop_handle = tokio::spawn(async move {
            submission_loop(session_for_loop, config, rx_sub)
                .instrument(info_span!("session_loop", thread_id = %thread_id))
                .await;
        });
        let codex = Codex {
            tx_sub,
            rx_event,
            session,
            session_loop_termination: session_loop_termination_from_handle(session_loop_handle),
        };

        Ok(CodexSpawnOk { codex, thread_id })
    }

    pub async fn submit(&self, op: Op) -> CodexResult<String> {
        self.submit_with_trace(op, None).await
    }

    pub async fn submit_with_trace(
        &self,
        op: Op,
        trace: Option<W3cTraceContext>,
    ) -> CodexResult<String> {
        let id = Uuid::now_v7().to_string();
        let sub = Submission {
            id: id.clone(),
            op,
            trace,
        };
        self.submit_with_id(sub).await?;
        Ok(id)
    }

    pub async fn submit_with_id(&self, mut sub: Submission) -> CodexResult<()> {
        if sub.trace.is_none() {
            sub.trace = current_span_w3c_trace_context();
        }
        self.tx_sub
            .send(sub)
            .await
            .map_err(|_| CodexErr::InternalAgentDied)?;
        Ok(())
    }
    pub async fn shutdown_and_wait(&self) -> CodexResult<()> {
        let session_loop_termination = self.session_loop_termination.clone();
        match self.submit(Op::Shutdown).await {
            Ok(_) => {}
            Err(CodexErr::InternalAgentDied) => {}
            Err(err) => return Err(err),
        }
        session_loop_termination.await;
        Ok(())
    }

    pub async fn next_event(&self) -> CodexResult<Event> {
        let event = self
            .rx_event
            .recv()
            .await
            .map_err(|_| CodexErr::InternalAgentDied)?;
        Ok(event)
    }

    pub async fn steer_input(
        &self,
        input: Vec<UserInput>,
        additional_context: BTreeMap<String, AdditionalContextEntry>,
        expected_turn_id: Option<&str>,
        responsesapi_client_metadata: Option<HashMap<String, String>>,
    ) -> Result<String, SteerInputError> {
        self.session
            .steer_input(
                input,
                additional_context,
                expected_turn_id,
                responsesapi_client_metadata,
            )
            .await
    }

    pub(crate) async fn set_app_server_client_info(
        &self,
        app_server_client_name: Option<String>,
        app_server_client_version: Option<String>,
    ) -> ConstraintResult<()> {
        self.session
            .update_settings(SessionSettingsUpdate {
                app_server_client_name,
                app_server_client_version,
                ..Default::default()
            })
            .await?;
        Ok(())
    }

    pub(crate) async fn thread_config_snapshot(&self) -> ThreadConfigSnapshot {
        let state = self.session.state.lock().await;
        state.session_configuration.thread_config_snapshot()
    }

    pub(crate) async fn thread_environment_selections(&self) -> Vec<TurnEnvironmentSelection> {
        let state = self.session.state.lock().await;
        state.session_configuration.environments.clone()
    }

    pub(crate) fn state_db(&self) -> Option<state_db::StateDbHandle> {
        self.session.state_db()
    }

    pub(crate) fn enabled(&self, feature: Feature) -> bool {
        self.session.features.enabled(feature)
    }
}

fn get_service_tier(
    configured_service_tier: Option<String>,
    model_info: &ModelInfo,
) -> Option<String> {
    configured_service_tier.filter(|service_tier| {
        service_tier == SERVICE_TIER_DEFAULT_REQUEST_VALUE
            || model_info.supports_service_tier(service_tier)
    })
}

pub(crate) fn session_loop_termination_from_handle(
    handle: JoinHandle<()>,
) -> SessionLoopTermination {
    async move {
        let _ = handle.await;
    }
    .boxed()
    .shared()
}

async fn thread_title_from_thread_store(
    live_thread: Option<&LiveThread>,
    thread_store: &Arc<dyn ThreadStore>,
    conversation_id: ThreadId,
) -> Option<String> {
    let thread = match live_thread {
        Some(live_thread) => live_thread.read_thread(true, false).await,
        None => {
            thread_store
                .read_thread(ReadThreadParams {
                    thread_id: conversation_id,
                    include_archived: true,
                    include_history: false,
                })
                .await
        }
    }
    .ok()?;

    let title = thread.name.as_deref()?.trim();
    (!title.is_empty() && thread.preview.trim() != title).then(|| title.to_string())
}

impl Session {
    pub(crate) fn state_db(&self) -> Option<state_db::StateDbHandle> {
        self.services.state_db.clone()
    }

    pub(crate) fn live_thread_for_persistence(
        &self,
        operation: &str,
    ) -> anyhow::Result<&LiveThread> {
        self.live_thread()
            .ok_or_else(|| anyhow::anyhow!("Session persistence is disabled; cannot {operation}."))
    }

    pub(crate) fn live_thread(&self) -> Option<&LiveThread> {
        self.services.live_thread.as_ref()
    }

    pub(crate) async fn flush_rollout(&self) -> std::io::Result<()> {
        if let Some(live_thread) = self.live_thread() {
            live_thread.flush().await.map_err(std::io::Error::other)
        } else {
            Ok(())
        }
    }

    pub(crate) async fn try_ensure_rollout_materialized(&self) -> std::io::Result<()> {
        if let Some(live_thread) = self.live_thread() {
            live_thread.persist().await.map_err(std::io::Error::other)?;
        }
        Ok(())
    }

    pub(crate) async fn ensure_rollout_materialized(&self) {
        if let Err(e) = self.try_ensure_rollout_materialized().await {
            warn!("failed to materialize thread persistence: {e}");
        }
    }

    fn next_internal_sub_id(&self) -> String {
        let id = self
            .next_internal_sub_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        format!("auto-compact-{id}")
    }

    pub(crate) async fn get_total_token_usage(&self) -> i64 {
        let state = self.state.lock().await;
        state.get_total_token_usage(state.server_reasoning_included())
    }

    pub(crate) async fn auto_compact_window_snapshot(&self) -> AutoCompactWindowSnapshot {
        let state = self.state.lock().await;
        state.auto_compact_window_snapshot()
    }

    pub(crate) async fn get_total_token_usage_breakdown(&self) -> TotalTokenUsageBreakdown {
        let state = self.state.lock().await;
        state.history.get_total_token_usage_breakdown()
    }

    pub(crate) async fn total_token_usage(&self) -> Option<TokenUsage> {
        let state = self.state.lock().await;
        state.token_info().map(|info| info.total_token_usage)
    }

    pub(crate) async fn token_usage_info(&self) -> Option<TokenUsageInfo> {
        let state = self.state.lock().await;
        state.token_info()
    }

    pub(crate) async fn get_estimated_token_count(
        &self,
        turn_context: &TurnContext,
    ) -> Option<i64> {
        let state = self.state.lock().await;
        state.history.estimate_token_count(turn_context)
    }

    pub(crate) async fn get_base_instructions(&self) -> BaseInstructions {
        let state = self.state.lock().await;
        BaseInstructions {
            text: state.session_configuration.base_instructions.clone(),
        }
    }

    async fn record_initial_history(&self, conversation_history: InitialHistory) {
        let turn_context = self.new_default_turn().await;
        let has_prior_user_turns = initial_history_has_prior_user_turns(&conversation_history);
        {
            let mut state = self.state.lock().await;
            state.set_next_turn_is_first(!has_prior_user_turns);
        }
        match conversation_history {
            InitialHistory::New | InitialHistory::Cleared => {
                self.set_previous_turn_settings(None).await;
            }
            InitialHistory::Resumed(resumed_history) => {
                let rollout_items = resumed_history.history;
                let previous_turn_settings = self
                    .apply_rollout_reconstruction(&turn_context, &rollout_items)
                    .await;

                let curr: &str = turn_context.model_info.slug.as_str();
                if let Some(prev) = previous_turn_settings
                    .as_ref()
                    .map(|settings| settings.model.as_str())
                    .filter(|model| *model != curr)
                {
                    warn!("resuming session with different model: previous={prev}, current={curr}");
                    self.send_event(
                        &turn_context,
                        EventMsg::Warning(WarningEvent {
                            message: format!(
                                "This session was recorded with model `{prev}` but is resuming with `{curr}`. \
                         Consider switching back to `{prev}` as it may affect Codex performance."
                            ),
                        }),
                    )
                    .await;
                }

                if let Some(info) = Self::last_token_info_from_rollout(&rollout_items) {
                    let mut state = self.state.lock().await;
                    state.set_token_info(Some(info));
                }
                let _ = self.flush_rollout().await;
            }
            InitialHistory::Forked(rollout_items) => {
                self.apply_rollout_reconstruction(&turn_context, &rollout_items)
                    .await;

                if let Some(info) = Self::last_token_info_from_rollout(&rollout_items) {
                    let mut state = self.state.lock().await;
                    state.set_token_info(Some(info));
                }

                if !rollout_items.is_empty() {
                    self.persist_rollout_items(&rollout_items).await;
                }

                self.ensure_rollout_materialized().await;
                let _ = self.flush_rollout().await;
            }
        }
    }

    async fn apply_rollout_reconstruction(
        &self,
        turn_context: &TurnContext,
        rollout_items: &[RolloutItem],
    ) -> Option<PreviousTurnSettings> {
        let reconstructed_rollout = self
            .reconstruct_history_from_rollout(turn_context, rollout_items)
            .await;
        let previous_turn_settings = reconstructed_rollout.previous_turn_settings.clone();
        self.replace_history(
            reconstructed_rollout.history,
            reconstructed_rollout.reference_context_item,
        )
        .await;
        let prefix_tokens = if matches!(
            turn_context.config.model_auto_compact_token_limit_scope,
            AutoCompactTokenLimitScope::BodyAfterPrefix
        ) {
            let history = self.clone_history().await;
            let base_instructions = self.get_base_instructions().await;
            history.estimate_token_count_with_base_instructions(&base_instructions)
        } else {
            None
        };
        if let Some(prefix_tokens) = prefix_tokens {
            self.set_auto_compact_window_estimated_prefill_for_scope(turn_context, prefix_tokens)
                .await;
        }
        self.set_previous_turn_settings(previous_turn_settings.clone())
            .await;
        previous_turn_settings
    }

    async fn set_auto_compact_window_estimated_prefill_for_scope(
        &self,
        turn_context: &TurnContext,
        tokens: i64,
    ) {
        if !matches!(
            turn_context.config.model_auto_compact_token_limit_scope,
            AutoCompactTokenLimitScope::BodyAfterPrefix
        ) {
            return;
        }

        let mut state = self.state.lock().await;
        state.set_auto_compact_window_estimated_prefill(tokens);
    }

    fn last_token_info_from_rollout(rollout_items: &[RolloutItem]) -> Option<TokenUsageInfo> {
        rollout_items.iter().rev().find_map(|item| match item {
            RolloutItem::EventMsg(EventMsg::TokenCount(ev)) => ev.info.clone(),
            _ => None,
        })
    }

    async fn previous_turn_settings(&self) -> Option<PreviousTurnSettings> {
        let state = self.state.lock().await;
        state.previous_turn_settings()
    }

    pub(crate) async fn set_previous_turn_settings(
        &self,
        previous_turn_settings: Option<PreviousTurnSettings>,
    ) {
        let mut state = self.state.lock().await;
        state.set_previous_turn_settings(previous_turn_settings);
    }

    fn maybe_refresh_shell_snapshot_for_cwd(
        &self,
        previous_cwd: &AbsolutePathBuf,
        next_cwd: &AbsolutePathBuf,
        codex_home: &AbsolutePathBuf,
    ) {
        if previous_cwd == next_cwd {
            return;
        }

        if !self.features.enabled(Feature::ShellSnapshot) {
            return;
        }

        ShellSnapshot::refresh_snapshot(
            codex_home.clone(),
            self.conversation_id,
            next_cwd.clone(),
            self.services.user_shell.as_ref().clone(),
            self.services.shell_snapshot_tx.clone(),
            self.services.session_telemetry.clone(),
            self.services.state_db.clone(),
        );
    }

    pub(crate) async fn update_settings(
        &self,
        updates: SessionSettingsUpdate,
    ) -> ConstraintResult<()> {
        let (previous_cwd, next_cwd, codex_home) = {
            let mut state = self.state.lock().await;
            let updated = match state.session_configuration.apply(&updates) {
                Ok(updated) => updated,
                Err(err) => {
                    warn!("rejected session settings update: {err}");
                    return Err(err);
                }
            };

            let previous_cwd = state.session_configuration.cwd.clone();
            let next_cwd = updated.cwd.clone();
            let codex_home = updated.codex_home.clone();
            state.session_configuration = updated;
            (previous_cwd, next_cwd, codex_home)
        };

        self.maybe_refresh_shell_snapshot_for_cwd(&previous_cwd, &next_cwd, &codex_home);

        Ok(())
    }

    pub(crate) async fn preview_settings(
        &self,
        updates: &SessionSettingsUpdate,
    ) -> ConstraintResult<ThreadConfigSnapshot> {
        let state = self.state.lock().await;
        state
            .session_configuration
            .apply(updates)
            .map(|configuration| configuration.thread_config_snapshot())
    }

    pub(crate) async fn set_session_startup_prewarm(
        &self,
        startup_prewarm: SessionStartupPrewarmHandle,
    ) {
        let mut state = self.state.lock().await;
        state.set_session_startup_prewarm(startup_prewarm);
    }

    pub(crate) async fn take_session_startup_prewarm(&self) -> Option<SessionStartupPrewarmHandle> {
        let mut state = self.state.lock().await;
        state.take_session_startup_prewarm()
    }

    pub(crate) async fn get_config(&self) -> std::sync::Arc<Config> {
        let state = self.state.lock().await;
        state
            .session_configuration
            .original_config_do_not_use
            .clone()
    }

    pub(crate) async fn provider(&self) -> ModelProviderInfo {
        let state = self.state.lock().await;
        state.session_configuration.provider.clone()
    }

    pub(crate) async fn refresh_runtime_config(&self, next_config: Config) {
        let _config = {
            let mut state = self.state.lock().await;
            let mut config = (*state.session_configuration.original_config_do_not_use).clone();
            config.config_layer_stack = config
                .config_layer_stack
                .with_user_layer_from(&next_config.config_layer_stack);
            let config = Arc::new(config);
            state.session_configuration.original_config_do_not_use = Arc::clone(&config);
            config
        };
    }

    pub(crate) async fn reload_user_config_layer(&self) {
        let config_toml_paths = {
            let state = self.state.lock().await;
            let config = &state.session_configuration.original_config_do_not_use;
            let user_config_paths = config
                .config_layer_stack
                .get_user_layers(ConfigLayerStackOrdering::LowestPrecedenceFirst, true)
                .into_iter()
                .filter_map(|layer| match &layer.name {
                    ConfigLayerSource::User { file, .. } => Some(file.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if user_config_paths.is_empty() {
                vec![
                    state
                        .session_configuration
                        .codex_home
                        .join(CONFIG_TOML_FILE),
                ]
            } else {
                user_config_paths
            }
        };

        let mut reloaded_user_configs = Vec::with_capacity(config_toml_paths.len());
        for config_toml_path in config_toml_paths {
            let user_config = match std::fs::read_to_string(&config_toml_path) {
                Ok(contents) => match toml::from_str::<toml::Value>(&contents) {
                    Ok(config) => config,
                    Err(err) => {
                        warn!("failed to parse user config while reloading layer: {err}");
                        return;
                    }
                },
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    toml::Value::Table(Default::default())
                }
                Err(err) => {
                    warn!("failed to read user config while reloading layer: {err}");
                    return;
                }
            };
            reloaded_user_configs.push((config_toml_path, user_config));
        }

        let next_config = {
            let state = self.state.lock().await;
            let mut config = (*state.session_configuration.original_config_do_not_use).clone();
            for (config_toml_path, user_config) in reloaded_user_configs {
                config.config_layer_stack = config
                    .config_layer_stack
                    .with_user_config(&config_toml_path, user_config);
            }
            config
        };
        self.refresh_runtime_config(next_config).await;
    }

    async fn build_settings_update_items(
        &self,
        reference_context_item: Option<&TurnContextItem>,
        current_context: &TurnContext,
    ) -> Vec<ResponseItem> {
        let previous_turn_settings = {
            let state = self.state.lock().await;
            state.previous_turn_settings()
        };
        let shell = self.user_shell();
        crate::context_manager::updates::build_settings_update_items(
            reference_context_item,
            previous_turn_settings.as_ref(),
            current_context,
            shell.as_ref(),
        )
    }

    pub(crate) async fn build_initial_context(
        &self,
        turn_context: &TurnContext,
    ) -> Vec<ResponseItem> {
        let mut developer_sections = Vec::new();
        let mut contextual_user_sections = Vec::new();

        let previous_turn_settings = {
            let state = self.state.lock().await;
            state.previous_turn_settings()
        };
        if let Some(model_switch_message) =
            crate::context_manager::updates::build_model_instructions_update_item(
                previous_turn_settings.as_ref(),
                turn_context,
            )
        {
            developer_sections.push(model_switch_message);
        }
        if let Some(developer_instructions) = turn_context.developer_instructions.as_deref()
            && !developer_instructions.is_empty()
        {
            developer_sections.push(developer_instructions.to_string());
        }
        if turn_context.config.include_environment_context {
            let shell = self.user_shell();
            contextual_user_sections.push(
                crate::context::EnvironmentContext::from_turn_context(turn_context, shell.as_ref())
                    .render(),
            );
        }

        let mut items = Vec::new();
        if let Some(developer_message) =
            crate::context_manager::updates::build_developer_update_item(developer_sections)
        {
            items.push(developer_message);
        }
        if let Some(contextual_user_message) =
            crate::context_manager::updates::build_contextual_user_message(contextual_user_sections)
        {
            items.push(contextual_user_message);
        }
        items
    }

    pub(crate) async fn record_context_updates_and_set_reference_context_item(
        &self,
        turn_context: &TurnContext,
    ) {
        let reference_context_item = {
            let state = self.state.lock().await;
            state.reference_context_item()
        };
        let context_items = if reference_context_item.is_none() {
            self.build_initial_context(turn_context).await
        } else {
            self.build_settings_update_items(reference_context_item.as_ref(), turn_context)
                .await
        };
        let turn_context_item = turn_context.to_turn_context_item();
        if !context_items.is_empty() {
            self.record_conversation_items(turn_context, &context_items)
                .await;
        }
        self.persist_rollout_items(&[RolloutItem::TurnContext(turn_context_item.clone())])
            .await;
        let mut state = self.state.lock().await;
        state.set_reference_context_item(Some(turn_context_item));
    }

    pub(crate) async fn record_conversation_items(
        &self,
        turn_context: &TurnContext,
        items: &[ResponseItem],
    ) {
        self.record_into_history(items, turn_context).await;
        self.persist_rollout_response_items(items).await;
        self.send_raw_response_items(turn_context, items).await;
    }

    pub(crate) async fn record_user_prompt_and_emit_turn_item(
        &self,
        turn_context: &TurnContext,
        user_input: &[UserInput],
    ) {
        let item = ResponseItem::from(ResponseInputItem::from(user_input.to_vec()));
        self.record_response_item_and_emit_turn_item(turn_context, item)
            .await;
    }

    pub(crate) async fn record_response_item_and_emit_turn_item(
        &self,
        turn_context: &TurnContext,
        item: ResponseItem,
    ) {
        self.record_conversation_items(turn_context, std::slice::from_ref(&item))
            .await;
        if let Some(turn_item) = crate::event_mapping::parse_turn_item(&item) {
            self.emit_turn_item_started(turn_context, &turn_item).await;
            self.emit_turn_item_completed(turn_context, turn_item).await;
        }
    }

    pub(crate) async fn record_into_history(
        &self,
        items: &[ResponseItem],
        turn_context: &TurnContext,
    ) {
        let mut state = self.state.lock().await;
        state.record_items(items.iter(), turn_context.truncation_policy);
    }

    pub(crate) async fn clone_history(&self) -> ContextManager {
        let state = self.state.lock().await;
        state.clone_history()
    }

    pub(crate) async fn reference_context_item(&self) -> Option<TurnContextItem> {
        let state = self.state.lock().await;
        state.reference_context_item()
    }

    pub(crate) async fn replace_history(
        &self,
        items: Vec<ResponseItem>,
        reference_context_item: Option<TurnContextItem>,
    ) {
        let mut state = self.state.lock().await;
        state.replace_history(items, reference_context_item);
    }

    pub(crate) async fn replace_compacted_history(
        &self,
        items: Vec<ResponseItem>,
        reference_context_item: Option<TurnContextItem>,
        compacted_item: CompactedItem,
    ) {
        {
            let mut state = self.state.lock().await;
            state.replace_history(items, reference_context_item.clone());
            state.start_next_auto_compact_window();
        }
        self.persist_rollout_items(&[RolloutItem::Compacted(compacted_item)])
            .await;
        if let Some(turn_context_item) = reference_context_item {
            self.persist_rollout_items(&[RolloutItem::TurnContext(turn_context_item)])
                .await;
        }
        self.services.model_client.advance_window_generation();
    }

    async fn persist_rollout_response_items(&self, items: &[ResponseItem]) {
        let rollout_items: Vec<RolloutItem> = items
            .iter()
            .cloned()
            .map(RolloutItem::ResponseItem)
            .collect();
        self.persist_rollout_items(&rollout_items).await;
    }

    async fn send_raw_response_items(&self, turn_context: &TurnContext, items: &[ResponseItem]) {
        for item in items {
            self.send_event(
                turn_context,
                EventMsg::RawResponseItem(RawResponseItemEvent { item: item.clone() }),
            )
            .await;
        }
    }

    pub(crate) async fn update_token_usage_info(
        &self,
        turn_context: &TurnContext,
        token_usage: Option<&TokenUsage>,
    ) {
        self.record_token_usage_info(turn_context, token_usage)
            .await;
        self.send_token_count_event(turn_context).await;
    }

    pub(crate) async fn record_token_usage_info(
        &self,
        turn_context: &TurnContext,
        token_usage: Option<&TokenUsage>,
    ) {
        if let Some(token_usage) = token_usage {
            let mut state = self.state.lock().await;
            state.update_token_info_from_usage(token_usage, turn_context.model_context_window());
            if matches!(
                turn_context.config.model_auto_compact_token_limit_scope,
                AutoCompactTokenLimitScope::BodyAfterPrefix
            ) {
                state.ensure_auto_compact_window_server_prefill_from_usage(token_usage);
            }
        }
    }

    pub(crate) async fn recompute_token_usage(&self, turn_context: &TurnContext) {
        let history = self.clone_history().await;
        let base_instructions = self.get_base_instructions().await;
        let Some(estimated_total_tokens) =
            history.estimate_token_count_with_base_instructions(&base_instructions)
        else {
            return;
        };
        {
            let mut state = self.state.lock().await;
            let mut info = state.token_info().unwrap_or(TokenUsageInfo {
                total_token_usage: TokenUsage::default(),
                last_token_usage: TokenUsage::default(),
                model_context_window: None,
            });
            info.last_token_usage = TokenUsage {
                input_tokens: 0,
                cached_input_tokens: 0,
                output_tokens: 0,
                reasoning_output_tokens: 0,
                total_tokens: estimated_total_tokens.max(0),
            };
            if let Some(model_context_window) = turn_context.model_context_window() {
                info.model_context_window = Some(model_context_window);
            }
            state.set_token_info(Some(info));
        }
        self.set_auto_compact_window_estimated_prefill_for_scope(
            turn_context,
            estimated_total_tokens,
        )
        .await;
        self.send_token_count_event(turn_context).await;
    }

    pub(crate) async fn update_rate_limits(
        &self,
        turn_context: &TurnContext,
        new_rate_limits: RateLimitSnapshot,
    ) {
        self.record_rate_limits_info(new_rate_limits).await;
        self.send_token_count_event(turn_context).await;
    }

    pub(crate) async fn record_rate_limits_info(&self, new_rate_limits: RateLimitSnapshot) {
        let mut state = self.state.lock().await;
        state.set_rate_limits(new_rate_limits);
    }

    pub(crate) async fn set_server_reasoning_included(&self, included: bool) {
        let mut state = self.state.lock().await;
        state.set_server_reasoning_included(included);
    }

    pub(crate) async fn send_token_count_event(&self, turn_context: &TurnContext) {
        let (info, rate_limits) = {
            let state = self.state.lock().await;
            state.token_info_and_rate_limits()
        };
        let event = EventMsg::TokenCount(TokenCountEvent { info, rate_limits });
        self.send_event(turn_context, event).await;
    }

    pub(crate) async fn send_event(&self, turn_context: &TurnContext, msg: EventMsg) {
        let event = Event {
            id: turn_context.sub_id.clone(),
            msg,
        };
        self.send_event_raw(event).await;
    }

    pub(crate) async fn send_event_raw(&self, event: Event) {
        let rollout_items = vec![RolloutItem::EventMsg(event.msg.clone())];
        self.persist_rollout_items(&rollout_items).await;
        self.deliver_event_raw(event).await;
    }

    pub(crate) async fn persist_rollout_items(&self, items: &[RolloutItem]) {
        let Some(live_thread) = self.live_thread() else {
            return;
        };
        if let Err(err) = live_thread.append_items(items).await {
            warn!("failed to persist rollout items: {err}");
        }
    }

    async fn deliver_event_raw(&self, event: Event) {
        if let Err(e) = self.tx_event.send(event).await {
            debug!("dropping event because channel is closed: {e}");
        }
    }

    pub(crate) async fn emit_turn_item_started(&self, turn_context: &TurnContext, item: &TurnItem) {
        self.send_event(
            turn_context,
            EventMsg::ItemStarted(ItemStartedEvent {
                thread_id: self.conversation_id,
                turn_id: turn_context.sub_id.clone(),
                item: item.clone(),
                started_at_ms: now_unix_timestamp_ms(),
            }),
        )
        .await;
    }

    pub(crate) async fn emit_turn_item_completed(
        &self,
        turn_context: &TurnContext,
        item: TurnItem,
    ) {
        record_turn_ttfm_metric(turn_context, &item).await;
        self.send_event(
            turn_context,
            EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id: self.conversation_id,
                turn_id: turn_context.sub_id.clone(),
                item,
                completed_at_ms: now_unix_timestamp_ms(),
            }),
        )
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state updates must remain atomic"
    )]

    pub async fn steer_input(
        self: &Arc<Self>,
        input: Vec<UserInput>,
        additional_context: BTreeMap<String, AdditionalContextEntry>,
        expected_turn_id: Option<&str>,
        responsesapi_client_metadata: Option<HashMap<String, String>>,
    ) -> Result<String, SteerInputError> {
        if input.is_empty() {
            return Err(SteerInputError::EmptyInput);
        }
        let turn_state = {
            let active = self.active_turn.lock().await;
            let Some(active_turn) = active.as_ref() else {
                return Err(SteerInputError::NoActiveTurn(input));
            };
            let Some(active_task) = active_turn.task.as_ref() else {
                return Err(SteerInputError::NoActiveTurn(input));
            };
            if let Some(expected_turn_id) = expected_turn_id
                && active_task.turn_context.sub_id != expected_turn_id
            {
                let actual = active_task.turn_context.sub_id.clone();
                return Err(SteerInputError::ExpectedTurnMismatch {
                    expected: expected_turn_id.to_string(),
                    actual,
                });
            }
            if active_task.kind == TaskKind::Compact {
                return Err(SteerInputError::ActiveTurnNotSteerable {
                    turn_kind: NonSteerableTurnKind::Compact,
                });
            }
            Arc::clone(&active_turn.turn_state)
        };
        if let Some(metadata) = responsesapi_client_metadata {
            let active = self.active_turn.lock().await;
            if let Some(active_task) = active.as_ref().and_then(|turn| turn.task.as_ref()) {
                active_task
                    .turn_context
                    .turn_metadata_state
                    .set_responsesapi_client_metadata(metadata);
            }
        }
        let turn_id = {
            let active = self.active_turn.lock().await;
            active
                .as_ref()
                .and_then(|turn| turn.task.as_ref())
                .map(|task| task.turn_context.sub_id.clone())
                .unwrap_or_default()
        };
        let additional_context_input = {
            let mut state = self.state.lock().await;
            state.additional_context.merge(additional_context)
        };
        let mut pending_input = additional_context_input
            .into_iter()
            .map(TurnInput::ResponseInputItem)
            .collect::<Vec<_>>();
        pending_input.push(TurnInput::UserInput(input));
        self.input_queue
            .extend_pending_input_for_turn_state(turn_state.as_ref(), pending_input)
            .await;
        Ok(expected_turn_id.map_or(turn_id, ToString::to_string))
    }

    pub(crate) async fn notify_dynamic_tool_response(
        &self,
        id: &str,
        response: DynamicToolResponse,
    ) {
        let tx = {
            let active = self.active_turn.lock().await;
            let Some(active_turn) = active.as_ref() else {
                return;
            };
            active_turn
                .turn_state
                .lock()
                .await
                .remove_pending_dynamic_tool(id)
        };
        if let Some(tx) = tx {
            let _ = tx.send(response);
        }
    }

    pub(crate) async fn set_total_tokens_full(&self, turn_context: &TurnContext) {
        let mut state = self.state.lock().await;
        let total_tokens = turn_context
            .model_context_window()
            .unwrap_or_else(|| state.get_total_token_usage(state.server_reasoning_included()));
        let mut info = state.token_info().unwrap_or(TokenUsageInfo {
            total_token_usage: TokenUsage::default(),
            last_token_usage: TokenUsage::default(),
            model_context_window: turn_context.model_context_window(),
        });
        info.last_token_usage.total_tokens = total_tokens;
        info.total_token_usage.total_tokens = total_tokens;
        state.set_token_info(Some(info));
    }

    pub(crate) async fn notify_stream_error(
        &self,
        turn_context: &TurnContext,
        message: String,
        _err: CodexErr,
    ) {
        self.send_event(
            turn_context,
            EventMsg::StreamError(StreamErrorEvent {
                message,
                codex_error_info: None,
                additional_details: None,
            }),
        )
        .await;
    }

    pub(crate) async fn maybe_warn_on_server_model_mismatch(
        &self,
        _turn_context: &TurnContext,
        _server_model: &str,
    ) -> bool {
        false
    }

    pub(crate) async fn emit_model_verification(
        &self,
        turn_context: &TurnContext,
        verifications: Vec<ModelVerification>,
    ) {
        self.send_event(
            turn_context,
            EventMsg::ModelVerification(ModelVerificationEvent { verifications }),
        )
        .await;
    }

    pub async fn interrupt_task(self: &Arc<Self>) {
        info!("interrupt received: abort current task, if any");
        let _had_active_turn = self.active_turn.lock().await.is_some();
        self.abort_all_tasks(TurnAbortReason::Interrupted).await;
    }

    pub(crate) fn user_shell(&self) -> Arc<shell::Shell> {
        Arc::clone(&self.services.user_shell)
    }
}
