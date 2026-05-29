use super::*;
use crate::config::GhostSnapshotConfig;
use codex_model_provider::SharedModelProvider;
use codex_model_provider::create_model_provider;
use codex_protocol::SessionId;
use codex_protocol::dynamic_tools::DynamicToolSpec;
use codex_protocol::protocol::TurnEnvironmentSelection;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

#[derive(Clone, Debug)]
pub(crate) struct TurnEnvironment {
    pub(crate) cwd: AbsolutePathBuf,
    pub(crate) shell: Option<String>,
}

impl TurnEnvironment {
    pub(crate) fn selection(&self) -> TurnEnvironmentSelection {
        TurnEnvironmentSelection {
            cwd: self.cwd.clone(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ResolvedTurnEnvironments {
    pub(crate) turn_environments: Vec<TurnEnvironment>,
    pub(crate) selections: Vec<TurnEnvironmentSelection>,
}

impl ResolvedTurnEnvironments {
    pub(crate) fn from_selections(selections: Vec<TurnEnvironmentSelection>) -> Self {
        let turn_environments = selections
            .iter()
            .map(|selection| TurnEnvironment {
                cwd: selection.cwd.clone(),
                shell: None,
            })
            .collect();
        Self {
            turn_environments,
            selections,
        }
    }

    pub(crate) fn primary(&self) -> Option<&TurnEnvironment> {
        self.turn_environments.first()
    }

    pub(crate) fn to_selections(&self) -> Vec<TurnEnvironmentSelection> {
        if self.turn_environments.is_empty() {
            return self.selections.clone();
        }
        self.turn_environments
            .iter()
            .map(TurnEnvironment::selection)
            .collect()
    }
}

#[derive(Debug)]
pub struct TurnContext {
    pub(crate) sub_id: String,
    pub(crate) trace_id: Option<String>,
    pub config: Arc<Config>,
    pub(crate) auth_manager: Option<Arc<AuthManager>>,
    pub(crate) model_info: ModelInfo,
    pub(crate) session_telemetry: SessionTelemetry,
    pub(crate) provider: SharedModelProvider,
    pub(crate) reasoning_effort: Option<ReasoningEffortConfig>,
    pub(crate) reasoning_summary: ReasoningSummaryConfig,
    pub(crate) session_source: SessionSource,
    pub(crate) environments: ResolvedTurnEnvironments,

    #[deprecated(note = "use the selected turn environment cwd instead")]
    pub(crate) cwd: AbsolutePathBuf,
    pub(crate) current_date: Option<String>,
    pub(crate) timezone: Option<String>,
    pub(crate) app_server_client_name: Option<String>,
    pub(crate) developer_instructions: Option<String>,
    pub(crate) compact_prompt: Option<String>,
    pub(crate) shell_environment_policy: ShellEnvironmentPolicy,
    pub features: ManagedFeatures,
    pub(crate) ghost_snapshot: GhostSnapshotConfig,
    pub(crate) final_output_json_schema: Option<Value>,
    pub(crate) codex_self_exe: Option<PathBuf>,
    pub(crate) truncation_policy: TruncationPolicy,
    pub(crate) dynamic_tools: Vec<DynamicToolSpec>,
    pub(crate) turn_metadata_state: Arc<TurnMetadataState>,
    pub(crate) turn_timing_state: Arc<TurnTimingState>,
    pub(crate) server_model_warning_emitted: AtomicBool,
    pub(crate) model_verification_emitted: AtomicBool,
}
impl TurnContext {
    pub(crate) fn effective_reasoning_effort(&self) -> Option<ReasoningEffortConfig> {
        if self.model_info.supports_reasoning_summaries {
            self.reasoning_effort
                .or(self.model_info.default_reasoning_level)
        } else {
            None
        }
    }

    pub(crate) fn effective_reasoning_effort_for_tracing(&self) -> String {
        self.effective_reasoning_effort()
            .map(|effort| effort.to_string())
            .unwrap_or_else(|| "default".to_string())
    }

    pub(crate) fn model_context_window(&self) -> Option<i64> {
        let effective_context_window_percent = self.model_info.effective_context_window_percent;
        self.model_info
            .resolved_context_window()
            .map(|context_window| {
                context_window.saturating_mul(effective_context_window_percent) / 100
            })
    }

    pub(crate) async fn with_model(
        &self,
        model: String,
        models_manager: &SharedModelsManager,
    ) -> Self {
        let mut config = (*self.config).clone();
        config.model = Some(model.clone());
        let model_info = models_manager
            .get_model_info(model.as_str(), &config.to_models_manager_config())
            .await;
        let truncation_policy = model_info.truncation_policy.into();
        let supported_reasoning_levels = model_info
            .supported_reasoning_levels
            .iter()
            .map(|preset| preset.effort)
            .collect::<Vec<_>>();
        let reasoning_effort = if let Some(current_reasoning_effort) = self.reasoning_effort {
            if supported_reasoning_levels.contains(&current_reasoning_effort) {
                Some(current_reasoning_effort)
            } else {
                supported_reasoning_levels
                    .get(supported_reasoning_levels.len().saturating_sub(1) / 2)
                    .copied()
                    .or(model_info.default_reasoning_level)
            }
        } else {
            supported_reasoning_levels
                .get(supported_reasoning_levels.len().saturating_sub(1) / 2)
                .copied()
                .or(model_info.default_reasoning_level)
        };
        config.model_reasoning_effort = reasoning_effort;

        let features = self.features.clone();

        Self {
            sub_id: self.sub_id.clone(),
            trace_id: self.trace_id.clone(),
            config: Arc::new(config),
            auth_manager: self.auth_manager.clone(),
            model_info: model_info.clone(),
            session_telemetry: self
                .session_telemetry
                .clone()
                .with_model(model.as_str(), model_info.slug.as_str()),
            provider: self.provider.clone(),
            reasoning_effort,
            reasoning_summary: self.reasoning_summary,
            session_source: self.session_source.clone(),
            environments: self.environments.clone(),
            #[allow(deprecated)]
            cwd: self.cwd.clone(),
            current_date: self.current_date.clone(),
            timezone: self.timezone.clone(),
            app_server_client_name: self.app_server_client_name.clone(),
            developer_instructions: self.developer_instructions.clone(),
            compact_prompt: self.compact_prompt.clone(),
            shell_environment_policy: self.shell_environment_policy.clone(),
            features,
            ghost_snapshot: self.ghost_snapshot.clone(),
            final_output_json_schema: self.final_output_json_schema.clone(),
            codex_self_exe: self.codex_self_exe.clone(),
            truncation_policy,
            dynamic_tools: self.dynamic_tools.clone(),
            turn_metadata_state: self.turn_metadata_state.clone(),
            turn_timing_state: Arc::clone(&self.turn_timing_state),
            server_model_warning_emitted: AtomicBool::new(
                self.server_model_warning_emitted.load(Ordering::Relaxed),
            ),
            model_verification_emitted: AtomicBool::new(
                self.model_verification_emitted.load(Ordering::Relaxed),
            ),
        }
    }

    pub(crate) fn to_turn_context_item(&self) -> TurnContextItem {
        TurnContextItem {
            turn_id: Some(self.sub_id.clone()),
            #[allow(deprecated)]
            cwd: self.cwd.to_path_buf(),
            current_date: self.current_date.clone(),
            timezone: self.timezone.clone(),
            model: self.model_info.slug.clone(),
            effort: self.reasoning_effort,
            summary: ReasoningSummaryConfig::Auto,
        }
    }
}

fn local_time_context() -> (String, String) {
    match iana_time_zone::get_timezone() {
        Ok(timezone) => (Local::now().format("%Y-%m-%d").to_string(), timezone),
        Err(_) => (
            Utc::now().format("%Y-%m-%d").to_string(),
            "Etc/UTC".to_string(),
        ),
    }
}

impl Session {
    pub(crate) fn build_per_turn_config(
        session_configuration: &SessionConfiguration,
        cwd: AbsolutePathBuf,
    ) -> Config {
        let config = session_configuration.original_config_do_not_use.clone();
        let mut per_turn_config = (*config).clone();
        per_turn_config.cwd = cwd;
        per_turn_config.workspace_roots = session_configuration.workspace_roots.clone();
        per_turn_config.model_reasoning_effort = session_configuration.model_reasoning_effort;
        per_turn_config.model_reasoning_summary = session_configuration.model_reasoning_summary;
        per_turn_config.service_tier = session_configuration.service_tier.clone();
        per_turn_config.features = config.features.clone();
        per_turn_config
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn make_turn_context(
        thread_id: ThreadId,
        session_id: SessionId,
        auth_manager: Option<Arc<AuthManager>>,
        session_telemetry: &SessionTelemetry,
        provider: ModelProviderInfo,
        session_configuration: &SessionConfiguration,
        _user_shell: &shell::Shell,
        _shell_zsh_path: Option<&PathBuf>,
        _main_execve_wrapper_exe: Option<&PathBuf>,
        per_turn_config: Config,
        model_info: ModelInfo,
        environments: ResolvedTurnEnvironments,
        cwd: AbsolutePathBuf,
        sub_id: String,
    ) -> TurnContext {
        let reasoning_effort = session_configuration.model_reasoning_effort;
        let reasoning_summary = session_configuration
            .model_reasoning_summary
            .unwrap_or(model_info.default_reasoning_summary);
        let session_telemetry = session_telemetry.clone().with_model(
            session_configuration.model.as_str(),
            model_info.slug.as_str(),
        );
        let session_source = session_configuration.session_source.clone();
        let auth_manager_for_context = auth_manager.clone();
        let provider_for_context = create_model_provider(provider, auth_manager);
        let session_telemetry_for_context = session_telemetry;
        let mut per_turn_config = per_turn_config;
        per_turn_config.service_tier = get_service_tier(
            per_turn_config.service_tier,
            per_turn_config.features.enabled(Feature::FastMode),
            &model_info,
        );
        let per_turn_config = Arc::new(per_turn_config);
        let turn_metadata_state = Arc::new(TurnMetadataState::new(
            session_id.to_string(),
            thread_id.to_string(),
            sub_id.clone(),
            cwd.clone(),
        ));
        let (current_date, timezone) = local_time_context();
        TurnContext {
            sub_id,
            trace_id: current_span_trace_id(),
            config: per_turn_config.clone(),
            auth_manager: auth_manager_for_context,
            model_info: model_info.clone(),
            session_telemetry: session_telemetry_for_context,
            provider: provider_for_context,
            reasoning_effort,
            reasoning_summary,
            session_source,
            environments,
            #[allow(deprecated)]
            cwd,
            current_date: Some(current_date),
            timezone: Some(timezone),
            app_server_client_name: session_configuration.app_server_client_name.clone(),
            developer_instructions: session_configuration.developer_instructions.clone(),
            compact_prompt: session_configuration.compact_prompt.clone(),
            shell_environment_policy: per_turn_config.shell_environment_policy.clone(),
            features: per_turn_config.features.clone(),
            ghost_snapshot: per_turn_config.ghost_snapshot.clone(),
            final_output_json_schema: None,
            codex_self_exe: per_turn_config.codex_self_exe.clone(),
            truncation_policy: model_info.truncation_policy.into(),
            dynamic_tools: session_configuration.dynamic_tools.clone(),
            turn_metadata_state,
            turn_timing_state: Arc::new(TurnTimingState::default()),
            server_model_warning_emitted: AtomicBool::new(false),
            model_verification_emitted: AtomicBool::new(false),
        }
    }

    pub(crate) async fn new_turn_with_sub_id(
        &self,
        sub_id: String,
        updates: SessionSettingsUpdate,
    ) -> CodexResult<Arc<TurnContext>> {
        let update_result: CodexResult<_> = {
            let mut state = self.state.lock().await;
            match state.session_configuration.clone().apply(&updates) {
                Ok(next) => {
                    let mut effective_environments = updates
                        .environments
                        .clone()
                        .unwrap_or_else(|| next.environments.clone());
                    if updates.environments.is_none() {
                        Self::overlay_runtime_cwd_on_primary_environment(
                            &mut effective_environments,
                            &next.cwd,
                        );
                    }
                    let turn_environments =
                        self.resolve_turn_environments(&effective_environments)?;
                    let previous_cwd = state.session_configuration.cwd.clone();
                    let codex_home = next.codex_home.clone();
                    let session_source = next.session_source.clone();
                    state.session_configuration = next.clone();
                    Ok((
                        next,
                        turn_environments,
                        previous_cwd,
                        codex_home,
                        session_source,
                    ))
                }
                Err(err) => Err(CodexErr::InvalidRequest(err.to_string())),
            }
        };

        let (session_configuration, turn_environments, previous_cwd, codex_home, _session_source) =
            match update_result {
                Ok(update) => update,
                Err(err) => {
                    let message = err.to_string();
                    self.send_event_raw(Event {
                        id: sub_id.clone(),
                        msg: EventMsg::Error(ErrorEvent {
                            message: message.clone(),
                            codex_error_info: Some(CodexErrorInfo::BadRequest),
                        }),
                    })
                    .await;
                    return Err(CodexErr::InvalidRequest(message));
                }
            };

        self.maybe_refresh_shell_snapshot_for_cwd(
            &previous_cwd,
            &session_configuration.cwd,
            &codex_home,
        );

        Ok(self
            .new_turn_from_configuration(
                sub_id,
                session_configuration,
                updates.final_output_json_schema,
                turn_environments,
            )
            .await)
    }

    fn resolve_turn_environments(
        &self,
        environments: &[TurnEnvironmentSelection],
    ) -> CodexResult<ResolvedTurnEnvironments> {
        Ok(ResolvedTurnEnvironments::from_selections(
            environments.to_vec(),
        ))
    }

    async fn new_turn_from_configuration(
        &self,
        sub_id: String,
        session_configuration: SessionConfiguration,
        final_output_json_schema: Option<Option<Value>>,
        turn_environments: ResolvedTurnEnvironments,
    ) -> Arc<TurnContext> {
        let primary_turn_environment = turn_environments.primary();
        let cwd = primary_turn_environment
            .map(|turn_environment| turn_environment.cwd.clone())
            .unwrap_or_else(|| session_configuration.cwd.clone());
        let per_turn_config = Self::build_per_turn_config(&session_configuration, cwd.clone());
        let model_info = self
            .services
            .models_manager
            .get_model_info(
                session_configuration.model.as_str(),
                &per_turn_config.to_models_manager_config(),
            )
            .await;
        let mut turn_context: TurnContext = Self::make_turn_context(
            self.thread_id(),
            self.session_id(),
            Some(Arc::clone(&self.services.auth_manager)),
            &self.services.session_telemetry,
            session_configuration.provider.clone(),
            &session_configuration,
            self.services.user_shell.as_ref(),
            self.services.shell_zsh_path.as_ref(),
            self.services.main_execve_wrapper_exe.as_ref(),
            per_turn_config,
            model_info,
            turn_environments,
            cwd,
            sub_id,
        );

        if let Some(final_schema) = final_output_json_schema {
            turn_context.final_output_json_schema = final_schema;
        }
        let turn_context = Arc::new(turn_context);
        turn_context.turn_metadata_state.spawn_git_enrichment_task();
        turn_context
    }

    pub(crate) async fn maybe_emit_unknown_model_warning_for_turn(&self, tc: &TurnContext) {
        if tc.model_info.used_fallback_model_metadata {
            self.send_event(
                tc,
                EventMsg::Warning(WarningEvent {
                    message: format!(
                        "Model metadata for `{}` not found. Defaulting to fallback metadata; this can degrade performance and cause issues.",
                        tc.model_info.slug
                    ),
                }),
            )
            .await
        }
    }

    pub(crate) async fn new_default_turn(&self) -> Arc<TurnContext> {
        self.new_default_turn_with_sub_id(self.next_internal_sub_id())
            .await
    }

    pub(crate) async fn new_default_turn_with_sub_id(&self, sub_id: String) -> Arc<TurnContext> {
        let session_configuration = {
            let state = self.state.lock().await;
            state.session_configuration.clone()
        };
        let mut effective_environments = session_configuration.environments.clone();
        Self::overlay_runtime_cwd_on_primary_environment(
            &mut effective_environments,
            &session_configuration.cwd,
        );
        let turn_environments = match self.resolve_turn_environments(&effective_environments) {
            Ok(turn_environments) => turn_environments,
            Err(err) => {
                warn!("failed to resolve stored session environments: {err}");
                ResolvedTurnEnvironments::default()
            }
        };

        self.new_turn_from_configuration(sub_id, session_configuration, None, turn_environments)
            .await
    }

    fn overlay_runtime_cwd_on_primary_environment(
        environments: &mut [TurnEnvironmentSelection],
        runtime_cwd: &AbsolutePathBuf,
    ) {
        if let Some(turn_environment) = environments.first_mut()
            && turn_environment.cwd != *runtime_cwd
        {
            turn_environment.cwd = runtime_cwd.clone();
        }
    }
}
