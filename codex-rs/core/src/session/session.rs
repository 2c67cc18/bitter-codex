use super::input_queue::InputQueue;
use super::*;
use crate::state::ActiveTurn;
use codex_protocol::SessionId;
use codex_protocol::config_types::SERVICE_TIER_DEFAULT_REQUEST_VALUE;
use codex_protocol::config_types::ServiceTier;
use codex_protocol::dynamic_tools::DynamicToolSpec;
use codex_protocol::protocol::TurnEnvironmentSelection;
use tokio::sync::watch;

pub(crate) struct Session {
    pub(crate) conversation_id: ThreadId,
    pub(super) tx_event: Sender<Event>,
    pub(super) state: Mutex<SessionState>,

    pub(super) features: ManagedFeatures,
    pub(crate) active_turn: Mutex<Option<ActiveTurn>>,
    pub(crate) input_queue: InputQueue,
    pub(crate) services: SessionServices,
    pub(super) next_internal_sub_id: AtomicU64,
}

#[derive(Clone)]
pub(crate) struct SessionConfiguration {
    pub(super) provider: ModelProviderInfo,
    pub(super) model: String,
    pub(super) model_reasoning_effort: Option<ReasoningEffortConfig>,
    pub(super) model_reasoning_summary: Option<ReasoningSummaryConfig>,
    pub(super) service_tier: Option<String>,

    pub(super) developer_instructions: Option<String>,

    pub(super) base_instructions: String,

    pub(super) compact_prompt: Option<String>,

    pub(super) cwd: AbsolutePathBuf,

    pub(super) workspace_roots: Vec<AbsolutePathBuf>,

    pub(super) codex_home: AbsolutePathBuf,

    pub(super) thread_name: Option<String>,

    pub(super) environments: Vec<TurnEnvironmentSelection>,

    pub(super) original_config_do_not_use: Arc<Config>,

    pub(super) metrics_service_name: Option<String>,
    pub(super) app_server_client_name: Option<String>,
    pub(super) app_server_client_version: Option<String>,

    pub(super) session_source: SessionSource,
    pub(super) dynamic_tools: Vec<DynamicToolSpec>,
    pub(super) persist_extended_history: bool,
    pub(super) inherited_shell_snapshot: Option<Arc<ShellSnapshot>>,
    pub(super) user_shell_override: Option<shell::Shell>,
}

impl SessionConfiguration {

    pub(super) fn thread_config_snapshot(&self) -> ThreadConfigSnapshot {
        ThreadConfigSnapshot {
            model: self.model.as_str().to_string(),
            model_provider_id: self.original_config_do_not_use.model_provider_id.clone(),
            service_tier: self.service_tier.clone(),
            cwd: self.cwd.clone(),
            workspace_roots: self.workspace_roots.clone(),
            ephemeral: self.original_config_do_not_use.ephemeral,
            reasoning_effort: self.model_reasoning_effort,
            reasoning_summary: self.model_reasoning_summary,
        }
    }

    pub(crate) fn apply(&self, updates: &SessionSettingsUpdate) -> ConstraintResult<Self> {
        let mut next_configuration = self.clone();
        if let Some(summary) = updates.reasoning_summary {
            next_configuration.model_reasoning_summary = Some(summary);
        }
        if let Some(service_tier) = updates.service_tier.clone() {
            next_configuration.service_tier = match service_tier {
                Some(service_tier) => Some(
                    ServiceTier::from_request_value(&service_tier)
                        .map_or(service_tier, |service_tier| {
                            service_tier.request_value().to_string()
                        }),
                ),
                None => Some(SERVICE_TIER_DEFAULT_REQUEST_VALUE.to_string()),
            };
        }

        let absolute_cwd = updates
            .cwd
            .as_ref()
            .map(|cwd| {
                AbsolutePathBuf::relative_to_current_dir(normalize_for_native_workdir(
                    cwd.as_path(),
                ))
                .unwrap_or_else(|e| {
                    warn!("failed to normalize update cwd: {cwd:?}: {e}");
                    self.cwd.clone()
                })
            })
            .unwrap_or_else(|| self.cwd.clone());

        let cwd_changed = absolute_cwd.as_path() != self.cwd.as_path();
        next_configuration.cwd = absolute_cwd;
        if let Some(workspace_roots) = updates.workspace_roots.clone() {
            next_configuration.workspace_roots = workspace_roots;
        } else if cwd_changed && self.workspace_roots.contains(&self.cwd) {
            let mut retargeted_workspace_roots =
                Vec::with_capacity(next_configuration.workspace_roots.len());
            for root in &self.workspace_roots {
                let root = if root == &self.cwd {
                    next_configuration.cwd.clone()
                } else {
                    root.clone()
                };
                if !retargeted_workspace_roots.contains(&root) {
                    retargeted_workspace_roots.push(root);
                }
            }
            next_configuration.workspace_roots = retargeted_workspace_roots;
        }
        if let Some(app_server_client_name) = updates.app_server_client_name.clone() {
            next_configuration.app_server_client_name = Some(app_server_client_name);
        }
        if let Some(app_server_client_version) = updates.app_server_client_version.clone() {
            next_configuration.app_server_client_version = Some(app_server_client_version);
        }
        Ok(next_configuration)
    }
}

#[derive(Default, Clone)]
pub(crate) struct SessionSettingsUpdate {
    pub(crate) cwd: Option<PathBuf>,
    pub(crate) workspace_roots: Option<Vec<AbsolutePathBuf>>,
    pub(crate) reasoning_summary: Option<ReasoningSummaryConfig>,
    pub(crate) service_tier: Option<Option<String>>,
    pub(crate) final_output_json_schema: Option<Option<Value>>,

    pub(crate) environments: Option<Vec<TurnEnvironmentSelection>>,
    pub(crate) app_server_client_name: Option<String>,
    pub(crate) app_server_client_version: Option<String>,
}

pub(crate) struct AppServerClientMetadata {
    pub(crate) client_name: Option<String>,
    pub(crate) client_version: Option<String>,
}

impl Session {
    pub(crate) fn thread_id(&self) -> ThreadId {
        self.conversation_id
    }

    pub(crate) fn session_id(&self) -> SessionId {
        self.services.session_id
    }

    #[instrument(name = "session_init", level = "info", skip_all)]
    #[allow(clippy::too_many_arguments)]
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "session initialization must serialize access through session-owned manager guards"
    )]
    pub(crate) async fn new(
        mut session_configuration: SessionConfiguration,
        config: Arc<Config>,
        installation_id: String,
        auth_manager: Arc<AuthManager>,
        models_manager: SharedModelsManager,
        tx_event: Sender<Event>,
        initial_history: InitialHistory,
        session_source: SessionSource,
        parent_session_id: Option<SessionId>,
        thread_store: Arc<dyn ThreadStore>,
    ) -> anyhow::Result<Arc<Self>> {
        debug!(
            "Configuring session: model={}; provider={:?}",
            session_configuration.model.as_str(),
            session_configuration.provider
        );
        let forked_from_id = initial_history.forked_from_id();

        let event_persistence_mode = if session_configuration.persist_extended_history {
            ThreadEventPersistenceMode::Extended
        } else {
            ThreadEventPersistenceMode::Limited
        };
        let thread_id = match &initial_history {
            InitialHistory::New | InitialHistory::Cleared | InitialHistory::Forked(_) => {
                ThreadId::default()
            }
            InitialHistory::Resumed(resumed_history) => resumed_history.conversation_id,
        };
        let window_generation = match &initial_history {
            InitialHistory::Resumed(resumed_history) => u64::try_from(
                resumed_history
                    .history
                    .iter()
                    .filter(|item| matches!(item, RolloutItem::Compacted(_)))
                    .count(),
            )
            .unwrap_or(u64::MAX),
            InitialHistory::New | InitialHistory::Cleared | InitialHistory::Forked(_) => 0,
        };

        let thread_persistence_fut = async {
            if config.ephemeral {
                Ok::<_, anyhow::Error>(None)
            } else {
                let live_thread = match &initial_history {
                    InitialHistory::New | InitialHistory::Cleared | InitialHistory::Forked(_) => {
                        LiveThread::create(
                            Arc::clone(&thread_store),
                            CreateThreadParams {
                                thread_id,
                                forked_from_id,
                                source: session_source,
                                base_instructions: BaseInstructions {
                                    text: session_configuration.base_instructions.clone(),
                                },
                                dynamic_tools: session_configuration.dynamic_tools.clone(),
                                metadata: ThreadPersistenceMetadata {
                                    cwd: Some(config.cwd.to_path_buf()),
                                    model_provider: config.model_provider_id.clone(),
                                },
                                event_persistence_mode,
                            },
                        )
                        .await?
                    }
                    InitialHistory::Resumed(resumed_history) => {
                        LiveThread::resume(
                            Arc::clone(&thread_store),
                            ResumeThreadParams {
                                thread_id: resumed_history.conversation_id,
                                rollout_path: resumed_history.rollout_path.clone(),
                                history: Some(resumed_history.history.clone()),
                                include_archived: true,
                                metadata: ThreadPersistenceMetadata {
                                    cwd: Some(config.cwd.to_path_buf()),
                                    model_provider: config.model_provider_id.clone(),
                                },
                                event_persistence_mode,
                            },
                        )
                        .await?
                    }
                };
                Ok(Some(live_thread))
            }
        }
        .instrument(info_span!(
            "session_init.thread_persistence",
            otel.name = "session_init.thread_persistence",
            session_init.ephemeral = config.ephemeral,
        ));
        let state_db_fut = async {
            if config.ephemeral {
                None
            } else if let Some(local_store) =
                thread_store.as_any().downcast_ref::<LocalThreadStore>()
            {
                local_store.state_db().await
            } else {
                None
            }
        }
        .instrument(info_span!(
            "session_init.state_db",
            otel.name = "session_init.state_db",
            session_init.ephemeral = config.ephemeral,
        ));

        let auth_manager_clone = Arc::clone(&auth_manager);
        let auth_fut = async move { auth_manager_clone.auth().await }.instrument(info_span!(
            "session_init.auth",
            otel.name = "session_init.auth",
        ));

        let (thread_persistence_result, state_db_ctx, auth) =
            tokio::join!(thread_persistence_fut, state_db_fut, auth_fut,);

        let mut live_thread_init =
            LiveThreadInitGuard::new(thread_persistence_result.map_err(|e| {
                error!("failed to initialize thread persistence: {e:#}");
                e
            })?);
        let session_result: anyhow::Result<Arc<Self>> = async {
            let rollout_path = if let Some(live_thread) = live_thread_init.as_ref() {
                live_thread.local_rollout_path().await?
            } else {
                None
            };

            let mut post_session_configured_events = Vec::<Event>::new();

            for message in &config.startup_warnings {
                post_session_configured_events.push(Event {
                    id: "".to_owned(),
                    msg: EventMsg::Warning(WarningEvent {
                        message: message.clone(),
                    }),
                });
            }
            let auth = auth.as_ref();
            let auth_mode = auth.map(CodexAuth::auth_mode).map(TelemetryAuthMode::from);
            let account_id = auth.and_then(CodexAuth::get_account_id);
            let account_email = auth.and_then(CodexAuth::get_account_email);
            let originator = originator().value;
            let terminal_type = "unknown".to_string();
            let session_model = session_configuration.model.as_str().to_string();
            let auth_env_telemetry = collect_auth_env_telemetry(
                &session_configuration.provider,
                auth_manager.codex_api_key_env_enabled(),
            );
            let mut session_telemetry = SessionTelemetry::new(
                thread_id,
                session_model.as_str(),
                session_model.as_str(),
                account_id.clone(),
                account_email.clone(),
                auth_mode,
                originator.clone(),
                config.otel.log_user_prompt,
                terminal_type,
                session_configuration.session_source.clone(),
            )
            .with_auth_env(auth_env_telemetry.to_otel_metadata());
            if let Some(service_name) = session_configuration.metrics_service_name.as_deref() {
                session_telemetry = session_telemetry.with_metrics_service_name(service_name);
            }
            config.features.emit_metrics(&session_telemetry);
            session_telemetry.counter(
                THREAD_STARTED_METRIC,
                 1,
                &[(
                    "is_git",
                    if get_git_repo_root(&session_configuration.cwd).is_some() {
                        "true"
                    } else {
                        "false"
                    },
                )],
            );

            session_telemetry.conversation_starts(
                config.model_provider.name.as_str(),
                session_configuration.model_reasoning_effort,
                config
                    .model_reasoning_summary
                    .unwrap_or(ReasoningSummaryConfig::Auto),
                config.model_context_window,
                config.model_auto_compact_token_limit,
            );

            let use_zsh_fork_shell = config.features.enabled(Feature::ShellZshFork);
            let mut default_shell = if let Some(user_shell_override) =
                session_configuration.user_shell_override.clone()
            {
                user_shell_override
            } else if use_zsh_fork_shell {
                let zsh_path = config.zsh_path.as_ref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "zsh fork feature enabled, but `zsh_path` is not configured; set `zsh_path` in config.toml"
                    )
                })?;
                let zsh_path = zsh_path.to_path_buf();
                shell::get_shell(shell::ShellType::Zsh, Some(&zsh_path)).ok_or_else(|| {
                    anyhow::anyhow!(
                        "zsh fork feature enabled, but zsh_path `{}` is not usable; set `zsh_path` to a valid zsh executable",
                        zsh_path.display()
                    )
                })?
            } else {
                shell::default_user_shell()
            };

            let shell_snapshot_tx = if config.features.enabled(Feature::ShellSnapshot) {
                if let Some(snapshot) = session_configuration.inherited_shell_snapshot.clone() {
                    let (tx, rx) = watch::channel(Some(snapshot));
                    default_shell.shell_snapshot = rx;
                    tx
                } else {
                    ShellSnapshot::start_snapshotting(
                        config.codex_home.clone(),
                        thread_id,
                        session_configuration.cwd.clone(),
                        &mut default_shell,
                        session_telemetry.clone(),
                        state_db_ctx.clone(),
                    )
                }
            } else {
                let (tx, rx) = watch::channel(None);
                default_shell.shell_snapshot = rx;
                tx
            };
            let thread_name =
                thread_title_from_thread_store(live_thread_init.as_ref(), &thread_store, thread_id)
                    .instrument(info_span!(
                        "session_init.thread_name_lookup",
                        otel.name = "session_init.thread_name_lookup",
                    ))
                    .await;
            session_configuration.thread_name = thread_name.clone();
            validate_config_lock_if_configured(&session_configuration).await?;
            export_config_lock_if_configured(&session_configuration, thread_id).await?;
            let state = SessionState::new(session_configuration.clone());
            let session_id = parent_session_id.unwrap_or_else(|| SessionId::from(thread_id));

            let services = SessionServices {







                unified_exec_manager: UnifiedExecProcessManager::new(
                    config.background_terminal_max_timeout,
                ),
                shell_zsh_path: config.zsh_path.clone(),
                main_execve_wrapper_exe: config.main_execve_wrapper_exe.clone(),
                user_shell: Arc::new(default_shell),
                shell_snapshot_tx,
                auth_manager: Arc::clone(&auth_manager),
                session_telemetry,
                models_manager: Arc::clone(&models_manager),
                session_id,
                state_db: state_db_ctx.clone(),
                live_thread: live_thread_init.as_ref().cloned(),
                model_client: ModelClient::new(
                    Some(Arc::clone(&auth_manager)),
                    session_id,
                    thread_id,
                    installation_id.clone(),
                    session_configuration.provider.clone(),
                    session_configuration.session_source.clone(),
                    config.model_verbosity,
                    config.features.enabled(Feature::EnableRequestCompression),
                    config.features.enabled(Feature::RuntimeMetrics),
                ),
            };
            services
                .model_client
                .set_window_generation(window_generation);
            let sess = Arc::new(Session {
                conversation_id: thread_id,
                tx_event: tx_event.clone(),
                state: Mutex::new(state),
                features: config.features.clone(),
                active_turn: Mutex::new(None),
                input_queue: InputQueue::new(),
                services,
                next_internal_sub_id: AtomicU64::new(0),
            });


            let initial_messages = initial_history.get_event_msgs();
            let events = std::iter::once(Event {
                id: INITIAL_SUBMIT_ID.to_owned(),
                msg: EventMsg::SessionConfigured(SessionConfiguredEvent {
                    session_id,
                    thread_id,
                    forked_from_id,
                    thread_name: session_configuration.thread_name.clone(),
                    model: session_configuration.model.as_str().to_string(),
                    model_provider_id: config.model_provider_id.clone(),
                    service_tier: session_configuration.service_tier.clone(),
                    cwd: session_configuration.cwd.clone(),
                    reasoning_effort: session_configuration.model_reasoning_effort,
                    initial_messages,
                    rollout_path,
                }),
            })
            .chain(post_session_configured_events.into_iter());
            for event in events {
                sess.send_event_raw(event).await;
            }

            sess.schedule_startup_prewarm(session_configuration.base_instructions.clone())
                .await;

            Box::pin(sess.record_initial_history(initial_history)).await;

            Ok(sess)
        }
        .await;
        match session_result {
            Ok(sess) => {
                live_thread_init.commit();
                Ok(sess)
            }
            Err(err) => {
                live_thread_init.discard().await;
                Err(err)
            }
        }
    }
}
