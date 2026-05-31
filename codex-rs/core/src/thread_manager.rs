use crate::codex_thread::CodexThread;
use crate::config::Config;
use crate::rollout::truncation;
use crate::session::Codex;
use crate::session::CodexSpawnArgs;
use crate::session::CodexSpawnOk;
use crate::session::INITIAL_SUBMIT_ID;
use crate::session::turn_context::ResolvedTurnEnvironments;
use crate::shell_snapshot::ShellSnapshot;
use crate::tasks::InterruptedTurnHistoryMarker;
use crate::tasks::interrupted_turn_history_marker;
use codex_app_server_protocol::ThreadHistoryBuilder;
use codex_app_server_protocol::TurnStatus;
use codex_login::AuthManager;
use codex_login::CodexAuth;
use codex_model_provider::create_model_provider;
use codex_model_provider_info::ModelProviderInfo;
use codex_model_provider_info::OPENAI_PROVIDER_ID;
use codex_models_manager::manager::RefreshStrategy;
use codex_models_manager::manager::SharedModelsManager;
use codex_protocol::SessionId;
use codex_protocol::ThreadId;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result as CodexResult;
use codex_protocol::openai_models::ModelPreset;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::InitialHistory;
use codex_protocol::protocol::ResumedHistory;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::protocol::SessionConfiguredEvent;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::TurnAbortReason;
use codex_protocol::protocol::TurnAbortedEvent;
use codex_protocol::protocol::TurnEnvironmentSelection;
use codex_protocol::protocol::W3cTraceContext;
use codex_rollout::state_db::StateDbHandle;
use codex_thread_store::LocalThreadStore;
use codex_thread_store::LocalThreadStoreConfig;
use codex_thread_store::ReadThreadByRolloutPathParams;
use codex_thread_store::StoredThread;
use codex_thread_store::ThreadMetadataPatch;
use codex_thread_store::ThreadStore;
use codex_thread_store::ThreadStoreError;
use codex_thread_store::UpdateThreadMetadataParams;
use codex_utils_absolute_path::AbsolutePathBuf;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::sync::broadcast;
use tracing::warn;

const THREAD_CREATED_CHANNEL_CAPACITY: usize = 1024;

static FORCE_TEST_THREAD_MANAGER_BEHAVIOR: AtomicBool = AtomicBool::new(false);

pub(crate) fn set_thread_manager_test_mode_for_tests(enabled: bool) {
    FORCE_TEST_THREAD_MANAGER_BEHAVIOR.store(enabled, Ordering::Relaxed);
}

struct TempCodexHomeGuard {
    path: PathBuf,
}

impl Drop for TempCodexHomeGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

pub struct NewThread {
    pub thread_id: ThreadId,
    pub thread: Arc<CodexThread>,
    pub session_configured: SessionConfiguredEvent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkSnapshot {
    TruncateBeforeNthUserMessage(usize),

    Interrupted,
}

impl From<usize> for ForkSnapshot {
    fn from(value: usize) -> Self {
        Self::TruncateBeforeNthUserMessage(value)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ThreadShutdownReport {
    pub completed: Vec<ThreadId>,
    pub submit_failed: Vec<ThreadId>,
    pub timed_out: Vec<ThreadId>,
}

enum ShutdownOutcome {
    Complete,
    SubmitFailed,
    TimedOut,
}

pub struct ThreadManager {
    state: Arc<ThreadManagerState>,
    _test_codex_home_guard: Option<TempCodexHomeGuard>,
}

pub struct StartThreadOptions {
    pub config: Config,
    pub initial_history: InitialHistory,
    pub session_source: Option<SessionSource>,
    pub dynamic_tools: Vec<codex_protocol::dynamic_tools::DynamicToolSpec>,
    pub persist_extended_history: bool,
    pub metrics_service_name: Option<String>,
    pub parent_trace: Option<W3cTraceContext>,
    pub environments: Vec<TurnEnvironmentSelection>,
}

pub(crate) struct ThreadManagerState {
    threads: Arc<RwLock<HashMap<ThreadId, Arc<CodexThread>>>>,
    thread_created_tx: broadcast::Sender<ThreadId>,
    auth_manager: Arc<AuthManager>,
    models_manager: SharedModelsManager,
    thread_store: Arc<dyn ThreadStore>,
    session_source: SessionSource,
    installation_id: String,
}

pub fn build_models_manager(
    config: &Config,
    auth_manager: Arc<AuthManager>,
) -> SharedModelsManager {
    let provider = create_model_provider(config.model_provider.clone(), Some(auth_manager));
    provider.models_manager(
        config.codex_home.to_path_buf(),
        config.model_catalog.clone(),
    )
}

pub fn thread_store_from_config(
    config: &Config,
    state_db: Option<StateDbHandle>,
) -> Arc<dyn ThreadStore> {
    Arc::new(LocalThreadStore::new(
        LocalThreadStoreConfig::from_config(config),
        state_db,
    ))
}

impl ThreadManager {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: &Config,
        auth_manager: Arc<AuthManager>,
        session_source: SessionSource,
        thread_store: Arc<dyn ThreadStore>,
        installation_id: String,
    ) -> Self {
        let (thread_created_tx, _) = broadcast::channel(THREAD_CREATED_CHANNEL_CAPACITY);
        Self {
            state: Arc::new(ThreadManagerState {
                threads: Arc::new(RwLock::new(HashMap::new())),
                thread_created_tx,
                models_manager: build_models_manager(config, auth_manager.clone()),
                thread_store,
                auth_manager,
                session_source,
                installation_id,
            }),
            _test_codex_home_guard: None,
        }
    }

    pub(crate) fn with_models_provider_for_tests(
        auth: CodexAuth,
        provider: ModelProviderInfo,
    ) -> Self {
        set_thread_manager_test_mode_for_tests(true);
        let codex_home = std::env::temp_dir().join(format!(
            "codex-thread-manager-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&codex_home)
            .unwrap_or_else(|err| panic!("temp codex home dir create failed: {err}"));
        let mut manager =
            Self::with_models_provider_and_home_for_tests(auth, provider, codex_home.clone());
        manager._test_codex_home_guard = Some(TempCodexHomeGuard { path: codex_home });
        manager
    }

    pub(crate) fn with_models_provider_and_home_for_tests(
        auth: CodexAuth,
        provider: ModelProviderInfo,
        codex_home: PathBuf,
    ) -> Self {
        Self::with_models_provider_home_and_state_for_tests(auth, provider, codex_home, None)
    }

    pub(crate) fn with_models_provider_home_and_state_for_tests(
        auth: CodexAuth,
        provider: ModelProviderInfo,
        codex_home: PathBuf,
        state_db: Option<StateDbHandle>,
    ) -> Self {
        set_thread_manager_test_mode_for_tests(true);
        let auth_manager = AuthManager::from_auth_for_testing(auth);
        let installation_id = uuid::Uuid::new_v4().to_string();
        let (thread_created_tx, _) = broadcast::channel(THREAD_CREATED_CHANNEL_CAPACITY);
        let thread_store: Arc<dyn ThreadStore> = Arc::new(LocalThreadStore::new(
            LocalThreadStoreConfig {
                codex_home: codex_home.clone(),
                sqlite_home: codex_home.clone(),
                default_model_provider_id: OPENAI_PROVIDER_ID.to_string(),
            },
            state_db.clone(),
        ));
        Self {
            state: Arc::new(ThreadManagerState {
                threads: Arc::new(RwLock::new(HashMap::new())),
                thread_created_tx,
                models_manager: create_model_provider(provider, Some(auth_manager.clone()))
                    .models_manager(codex_home, None),
                thread_store,
                auth_manager,
                session_source: SessionSource::Exec,
                installation_id,
            }),
            _test_codex_home_guard: None,
        }
    }

    pub fn session_source(&self) -> SessionSource {
        self.state.session_source.clone()
    }

    pub fn auth_manager(&self) -> Arc<AuthManager> {
        self.state.auth_manager.clone()
    }

    pub fn default_environment_selections(
        &self,
        cwd: &AbsolutePathBuf,
    ) -> Vec<TurnEnvironmentSelection> {
        vec![TurnEnvironmentSelection { cwd: cwd.clone() }]
    }

    pub fn validate_environment_selections(
        &self,
        _environments: &[TurnEnvironmentSelection],
    ) -> CodexResult<()> {
        Ok(())
    }

    pub fn get_models_manager(&self) -> SharedModelsManager {
        self.state.models_manager.clone()
    }

    pub async fn list_models(&self, refresh_strategy: RefreshStrategy) -> Vec<ModelPreset> {
        self.state
            .models_manager
            .list_models(refresh_strategy)
            .await
    }

    pub async fn list_thread_ids(&self) -> Vec<ThreadId> {
        self.state.list_thread_ids().await
    }

    pub fn subscribe_thread_created(&self) -> broadcast::Receiver<ThreadId> {
        self.state.thread_created_tx.subscribe()
    }

    pub async fn get_thread(&self, thread_id: ThreadId) -> CodexResult<Arc<CodexThread>> {
        self.state.get_thread(thread_id).await
    }

    pub async fn update_thread_metadata(
        &self,
        thread_id: ThreadId,
        patch: ThreadMetadataPatch,
        include_archived: bool,
    ) -> CodexResult<StoredThread> {
        if let Ok(thread) = self.get_thread(thread_id).await {
            if thread.config_snapshot().await.ephemeral {
                return Err(CodexErr::InvalidRequest(format!(
                    "ephemeral thread does not support metadata updates: {thread_id}"
                )));
            }
            return thread
                .update_thread_metadata(patch, include_archived)
                .await
                .map_err(|err| thread_store_metadata_update_error(thread_id, err));
        }
        self.state
            .thread_store
            .update_thread_metadata(UpdateThreadMetadataParams {
                thread_id,
                patch,
                include_archived,
            })
            .await
            .map_err(|err| match err {
                ThreadStoreError::ThreadNotFound { thread_id } => {
                    CodexErr::ThreadNotFound(thread_id)
                }
                err => thread_store_metadata_update_error(thread_id, err),
            })
    }
    pub async fn start_thread(&self, config: Config) -> CodexResult<NewThread> {
        Box::pin(self.start_thread_with_tools(config, Vec::new(), false)).await
    }

    pub async fn start_thread_with_tools(
        &self,
        config: Config,
        dynamic_tools: Vec<codex_protocol::dynamic_tools::DynamicToolSpec>,
        persist_extended_history: bool,
    ) -> CodexResult<NewThread> {
        let environments = Vec::new();
        Box::pin(self.start_thread_with_options(StartThreadOptions {
            config,
            initial_history: InitialHistory::New,
            session_source: None,
            dynamic_tools,
            persist_extended_history,
            metrics_service_name: None,
            parent_trace: None,
            environments,
        }))
        .await
    }

    pub async fn start_thread_with_options(
        &self,
        options: StartThreadOptions,
    ) -> CodexResult<NewThread> {
        self.start_thread_with_options_and_fork_source(options, /*forked_from_thread_id*/ None)
            .await
    }

    async fn start_thread_with_options_and_fork_source(
        &self,
        options: StartThreadOptions,
        forked_from_thread_id: Option<ThreadId>,
    ) -> CodexResult<NewThread> {
        let session_source = options
            .session_source
            .unwrap_or_else(|| self.state.session_source.clone());
        Box::pin(self.state.spawn_thread_with_source(
            options.config,
            options.initial_history,
            Arc::clone(&self.state.auth_manager),
            None,
            session_source,
            forked_from_thread_id,
            options.dynamic_tools,
            options.persist_extended_history,
            options.metrics_service_name,
            None,
            options.parent_trace,
            options.environments,
            None,
        ))
        .await
    }
    pub async fn resume_thread_from_rollout(
        &self,
        config: Config,
        rollout_path: PathBuf,
        auth_manager: Arc<AuthManager>,
        parent_trace: Option<W3cTraceContext>,
    ) -> CodexResult<NewThread> {
        let initial_history = self.initial_history_from_rollout_path(rollout_path).await?;
        Box::pin(self.resume_thread_with_history(
            config,
            initial_history,
            auth_manager,
            false,
            parent_trace,
        ))
        .await
    }

    pub async fn resume_thread_with_history(
        &self,
        config: Config,
        initial_history: InitialHistory,
        auth_manager: Arc<AuthManager>,
        persist_extended_history: bool,
        parent_trace: Option<W3cTraceContext>,
    ) -> CodexResult<NewThread> {
        let environments = Vec::new();
        Box::pin(self.state.spawn_thread(
            config,
            initial_history,
            auth_manager,
            None,
            None,
            Vec::new(),
            persist_extended_history,
            None,
            parent_trace,
            environments,
            None,
        ))
        .await
    }

    pub(crate) async fn start_thread_with_user_shell_override_for_tests(
        &self,
        config: Config,
        user_shell_override: crate::shell::Shell,
    ) -> CodexResult<NewThread> {
        let environments = Vec::new();
        Box::pin(self.state.spawn_thread(
            config,
            InitialHistory::New,
            Arc::clone(&self.state.auth_manager),
            None,
            None,
            Vec::new(),
            false,
            None,
            None,
            environments,
            Some(user_shell_override),
        ))
        .await
    }

    pub(crate) async fn resume_thread_from_rollout_with_user_shell_override_for_tests(
        &self,
        config: Config,
        rollout_path: PathBuf,
        auth_manager: Arc<AuthManager>,
        user_shell_override: crate::shell::Shell,
    ) -> CodexResult<NewThread> {
        let initial_history = self.initial_history_from_rollout_path(rollout_path).await?;
        let environments = Vec::new();
        Box::pin(self.state.spawn_thread(
            config,
            initial_history,
            auth_manager,
            None,
            None,
            Vec::new(),
            false,
            None,
            None,
            environments,
            Some(user_shell_override),
        ))
        .await
    }

    pub async fn remove_thread(&self, thread_id: &ThreadId) -> Option<Arc<CodexThread>> {
        self.state.threads.write().await.remove(thread_id)
    }

    pub async fn shutdown_all_threads_bounded(&self, timeout: Duration) -> ThreadShutdownReport {
        let threads = {
            let threads = self.state.threads.read().await;
            threads
                .iter()
                .map(|(thread_id, thread)| (*thread_id, Arc::clone(thread)))
                .collect::<Vec<_>>()
        };

        let mut shutdowns = threads
            .into_iter()
            .map(|(thread_id, thread)| async move {
                let outcome = match tokio::time::timeout(timeout, thread.shutdown_and_wait()).await
                {
                    Ok(Ok(())) => ShutdownOutcome::Complete,
                    Ok(Err(_)) => ShutdownOutcome::SubmitFailed,
                    Err(_) => ShutdownOutcome::TimedOut,
                };
                (thread_id, outcome)
            })
            .collect::<FuturesUnordered<_>>();
        let mut report = ThreadShutdownReport::default();

        while let Some((thread_id, outcome)) = shutdowns.next().await {
            match outcome {
                ShutdownOutcome::Complete => report.completed.push(thread_id),
                ShutdownOutcome::SubmitFailed => report.submit_failed.push(thread_id),
                ShutdownOutcome::TimedOut => report.timed_out.push(thread_id),
            }
        }

        let mut tracked_threads = self.state.threads.write().await;
        for thread_id in &report.completed {
            tracked_threads.remove(thread_id);
        }

        report
            .completed
            .sort_by_key(std::string::ToString::to_string);
        report
            .submit_failed
            .sort_by_key(std::string::ToString::to_string);
        report
            .timed_out
            .sort_by_key(std::string::ToString::to_string);
        report
    }

    pub async fn fork_thread<S>(
        &self,
        snapshot: S,
        config: Config,
        path: PathBuf,
        persist_extended_history: bool,
        parent_trace: Option<W3cTraceContext>,
    ) -> CodexResult<NewThread>
    where
        S: Into<ForkSnapshot>,
    {
        let snapshot = snapshot.into();
        let history = self.initial_history_from_rollout_path(path).await?;
        self.fork_thread_from_history(
            snapshot,
            config,
            history,
            persist_extended_history,
            parent_trace,
        )
        .await
    }

    async fn initial_history_from_rollout_path(
        &self,
        rollout_path: PathBuf,
    ) -> CodexResult<InitialHistory> {
        let requested_rollout_path = rollout_path.clone();
        let stored_thread = self
            .state
            .thread_store
            .read_thread_by_rollout_path(ReadThreadByRolloutPathParams {
                rollout_path,
                include_archived: true,
                include_history: true,
            })
            .await
            .map_err(thread_store_rollout_read_error)?;
        stored_thread_to_initial_history(stored_thread, Some(requested_rollout_path))
    }

    pub async fn fork_thread_from_history<S>(
        &self,
        snapshot: S,
        config: Config,
        history: InitialHistory,
        persist_extended_history: bool,
        parent_trace: Option<W3cTraceContext>,
    ) -> CodexResult<NewThread>
    where
        S: Into<ForkSnapshot>,
    {
        self.fork_thread_with_initial_history(
            snapshot.into(),
            config,
            history,
            persist_extended_history,
            parent_trace,
        )
        .await
    }

    async fn fork_thread_with_initial_history(
        &self,
        snapshot: ForkSnapshot,
        config: Config,
        history: InitialHistory,
        persist_extended_history: bool,
        parent_trace: Option<W3cTraceContext>,
    ) -> CodexResult<NewThread> {
        let forked_from_thread_id = match &history {
            InitialHistory::Resumed(resumed) => Some(resumed.conversation_id),
            InitialHistory::Forked(_) => history.forked_from_id(),
            InitialHistory::New | InitialHistory::Cleared => None,
        };
        let interrupted_marker = InterruptedTurnHistoryMarker::from_config();
        let history = fork_history_from_snapshot(snapshot, history, interrupted_marker);
        let environments = Vec::new();
        Box::pin(self.state.spawn_thread(
            config,
            history,
            Arc::clone(&self.state.auth_manager),
            None,
            forked_from_thread_id,
            Vec::new(),
            persist_extended_history,
            None,
            parent_trace,
            environments,
            None,
        ))
        .await
    }
}

impl ThreadManagerState {
    pub(crate) async fn list_thread_ids(&self) -> Vec<ThreadId> {
        self.threads
            .read()
            .await
            .iter()
            .map(|(thread_id, _)| *thread_id)
            .collect()
    }

    pub(crate) async fn get_thread(&self, thread_id: ThreadId) -> CodexResult<Arc<CodexThread>> {
        let threads = self.threads.read().await;
        threads
            .get(&thread_id)
            .cloned()
            .ok_or(CodexErr::ThreadNotFound(thread_id))
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn spawn_thread(
        &self,
        config: Config,
        initial_history: InitialHistory,
        auth_manager: Arc<AuthManager>,
        parent_session_id: Option<SessionId>,
        forked_from_thread_id: Option<ThreadId>,
        dynamic_tools: Vec<codex_protocol::dynamic_tools::DynamicToolSpec>,
        persist_extended_history: bool,
        metrics_service_name: Option<String>,
        parent_trace: Option<W3cTraceContext>,
        environments: Vec<TurnEnvironmentSelection>,
        user_shell_override: Option<crate::shell::Shell>,
    ) -> CodexResult<NewThread> {
        Box::pin(self.spawn_thread_with_source(
            config,
            initial_history,
            auth_manager,
            parent_session_id,
            self.session_source.clone(),
            forked_from_thread_id,
            dynamic_tools,
            persist_extended_history,
            metrics_service_name,
            None,
            parent_trace,
            environments,
            user_shell_override,
        ))
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn spawn_thread_with_source(
        &self,
        config: Config,
        initial_history: InitialHistory,
        auth_manager: Arc<AuthManager>,
        parent_session_id: Option<SessionId>,
        session_source: SessionSource,
        forked_from_thread_id: Option<ThreadId>,
        dynamic_tools: Vec<codex_protocol::dynamic_tools::DynamicToolSpec>,
        persist_extended_history: bool,
        metrics_service_name: Option<String>,
        inherited_shell_snapshot: Option<Arc<ShellSnapshot>>,
        parent_trace: Option<W3cTraceContext>,
        environments: Vec<TurnEnvironmentSelection>,
        user_shell_override: Option<crate::shell::Shell>,
    ) -> CodexResult<NewThread> {
        let _is_resumed_thread = matches!(&initial_history, InitialHistory::Resumed(_));
        if let InitialHistory::Resumed(resumed) = &initial_history {
            let mut threads = self.threads.write().await;
            if let Some(thread) = threads.get(&resumed.conversation_id).cloned() {
                if thread.is_running() {
                    if let Some(requested_rollout_path) = resumed.rollout_path.as_deref()
                        && thread.rollout_path().as_deref() != Some(requested_rollout_path)
                    {
                        return Err(CodexErr::InvalidRequest(format!(
                            "thread {} is already running with a different rollout path",
                            resumed.conversation_id
                        )));
                    }
                    return Ok(NewThread {
                        thread_id: resumed.conversation_id,
                        session_configured: thread.session_configured(),
                        thread,
                    });
                }
                threads.remove(&resumed.conversation_id);
            }
        }
        let environment_selections = ResolvedTurnEnvironments::from_selections(environments);

        let tracked_session_source = session_source.clone();
        let CodexSpawnOk {
            codex, thread_id, ..
        } = Codex::spawn(CodexSpawnArgs {
            config,
            installation_id: self.installation_id.clone(),
            auth_manager,
            models_manager: Arc::clone(&self.models_manager),
            conversation_history: initial_history,
            dynamic_tools,
            parent_session_id,
            forked_from_thread_id,
            persist_extended_history,
            metrics_service_name,
            inherited_shell_snapshot,
            user_shell_override,
            parent_trace,
            session_source: tracked_session_source.clone(),
            environment_selections,
            thread_store: Arc::clone(&self.thread_store),
        })
        .await?;
        let new_thread = self.finalize_thread_spawn(codex, thread_id).await?;
        Ok(new_thread)
    }

    async fn finalize_thread_spawn(
        &self,
        codex: Codex,
        thread_id: ThreadId,
    ) -> CodexResult<NewThread> {
        let event = codex.next_event().await?;
        let session_configured = match event {
            Event {
                id,
                msg: EventMsg::SessionConfigured(session_configured),
            } if id == INITIAL_SUBMIT_ID => session_configured,
            _ => {
                return Err(CodexErr::SessionConfiguredNotFirstEvent);
            }
        };

        {
            let mut threads = self.threads.write().await;
            if let std::collections::hash_map::Entry::Vacant(e) = threads.entry(thread_id) {
                let thread = Arc::new(CodexThread::new(
                    codex,
                    session_configured.clone(),
                    session_configured.rollout_path.clone(),
                ));
                e.insert(thread.clone());
                return Ok(NewThread {
                    thread_id,
                    thread,
                    session_configured,
                });
            }
        }

        if let Err(err) = codex.shutdown_and_wait().await {
            warn!("failed to shut down duplicate thread {thread_id}: {err}");
        }
        Err(CodexErr::InvalidRequest(format!(
            "thread {thread_id} is already running"
        )))
    }
}

fn stored_thread_to_initial_history(
    stored_thread: StoredThread,
    rollout_path: Option<PathBuf>,
) -> CodexResult<InitialHistory> {
    let thread_id = stored_thread.thread_id;
    let history = stored_thread.history.ok_or_else(|| {
        CodexErr::Fatal(format!(
            "thread {thread_id} did not include persisted history"
        ))
    })?;
    Ok(InitialHistory::Resumed(ResumedHistory {
        conversation_id: thread_id,
        history: history.items,
        rollout_path: rollout_path.or(stored_thread.rollout_path),
    }))
}

fn thread_store_rollout_read_error(err: ThreadStoreError) -> CodexErr {
    match err {
        ThreadStoreError::ThreadNotFound { thread_id } => CodexErr::ThreadNotFound(thread_id),
        ThreadStoreError::InvalidRequest { message } => CodexErr::InvalidRequest(message),
        err => CodexErr::Fatal(format!("failed to read thread by rollout path: {err}")),
    }
}

fn thread_store_metadata_update_error(thread_id: ThreadId, err: ThreadStoreError) -> CodexErr {
    match err {
        ThreadStoreError::ThreadNotFound { thread_id } => CodexErr::ThreadNotFound(thread_id),
        ThreadStoreError::InvalidRequest { message } => CodexErr::InvalidRequest(message),
        ThreadStoreError::Unsupported { operation } => CodexErr::UnsupportedOperation(format!(
            "thread metadata update is not supported by this store: {operation}"
        )),
        err => CodexErr::Fatal(format!(
            "failed to update thread metadata {thread_id}: {err}"
        )),
    }
}

fn truncate_before_nth_user_message(
    history: InitialHistory,
    n: usize,
    snapshot_state: &SnapshotTurnState,
) -> InitialHistory {
    let items: Vec<RolloutItem> = history.get_rollout_items();
    let user_positions = truncation::user_message_positions_in_rollout(&items);
    let rolled = if snapshot_state.ends_mid_turn && n >= user_positions.len() {
        if let Some(cut_idx) = snapshot_state
            .active_turn_start_index
            .or_else(|| user_positions.last().copied())
        {
            items[..cut_idx].to_vec()
        } else {
            items
        }
    } else {
        truncation::truncate_rollout_before_nth_user_message_from_start(&items, n)
    };

    if rolled.is_empty() {
        InitialHistory::New
    } else {
        InitialHistory::Forked(rolled)
    }
}

#[derive(Debug, Eq, PartialEq)]
struct SnapshotTurnState {
    ends_mid_turn: bool,
    active_turn_id: Option<String>,
    active_turn_start_index: Option<usize>,
}

fn snapshot_turn_state(history: &InitialHistory) -> SnapshotTurnState {
    let rollout_items = history.get_rollout_items();
    let mut builder = ThreadHistoryBuilder::new();
    for item in &rollout_items {
        builder.handle_rollout_item(item);
    }
    let active_turn_id = builder.active_turn_id_if_explicit();
    if builder.has_active_turn() && active_turn_id.is_some() {
        let active_turn_snapshot = builder.active_turn_snapshot();
        if active_turn_snapshot
            .as_ref()
            .is_some_and(|turn| turn.status != TurnStatus::InProgress)
        {
            return SnapshotTurnState {
                ends_mid_turn: false,
                active_turn_id: None,
                active_turn_start_index: None,
            };
        }

        return SnapshotTurnState {
            ends_mid_turn: true,
            active_turn_id,
            active_turn_start_index: builder.active_turn_start_index(),
        };
    }

    let Some(last_user_position) = truncation::user_message_positions_in_rollout(&rollout_items)
        .last()
        .copied()
    else {
        return SnapshotTurnState {
            ends_mid_turn: false,
            active_turn_id: None,
            active_turn_start_index: None,
        };
    };

    SnapshotTurnState {
        ends_mid_turn: !rollout_items[last_user_position + 1..].iter().any(|item| {
            matches!(
                item,
                RolloutItem::EventMsg(EventMsg::TurnComplete(_) | EventMsg::TurnAborted(_))
            )
        }),
        active_turn_id: None,
        active_turn_start_index: None,
    }
}

fn fork_history_from_snapshot(
    snapshot: ForkSnapshot,
    history: InitialHistory,
    interrupted_marker: InterruptedTurnHistoryMarker,
) -> InitialHistory {
    let snapshot_state = snapshot_turn_state(&history);
    match snapshot {
        ForkSnapshot::TruncateBeforeNthUserMessage(nth_user_message) => {
            truncate_before_nth_user_message(history, nth_user_message, &snapshot_state)
        }
        ForkSnapshot::Interrupted => {
            let history = match history {
                InitialHistory::New => InitialHistory::New,
                InitialHistory::Cleared => InitialHistory::Cleared,
                InitialHistory::Forked(history) => InitialHistory::Forked(history),
                InitialHistory::Resumed(resumed) => InitialHistory::Forked(resumed.history),
            };
            if snapshot_state.ends_mid_turn {
                append_interrupted_boundary(
                    history,
                    snapshot_state.active_turn_id,
                    interrupted_marker,
                )
            } else {
                history
            }
        }
    }
}

fn append_interrupted_boundary(
    history: InitialHistory,
    turn_id: Option<String>,
    interrupted_marker: InterruptedTurnHistoryMarker,
) -> InitialHistory {
    let aborted_event = RolloutItem::EventMsg(EventMsg::TurnAborted(TurnAbortedEvent {
        turn_id,
        reason: TurnAbortReason::Interrupted,
        completed_at: None,
        duration_ms: None,
    }));

    match history {
        InitialHistory::New | InitialHistory::Cleared => {
            let mut history = Vec::new();
            if let Some(marker) = interrupted_turn_history_marker(interrupted_marker) {
                history.push(RolloutItem::ResponseItem(marker));
            }
            history.push(aborted_event);
            InitialHistory::Forked(history)
        }
        InitialHistory::Forked(mut history) => {
            if let Some(marker) = interrupted_turn_history_marker(interrupted_marker) {
                history.push(RolloutItem::ResponseItem(marker));
            }
            history.push(aborted_event);
            InitialHistory::Forked(history)
        }
        InitialHistory::Resumed(mut resumed) => {
            if let Some(marker) = interrupted_turn_history_marker(interrupted_marker) {
                resumed.history.push(RolloutItem::ResponseItem(marker));
            }
            resumed.history.push(aborted_event);
            InitialHistory::Forked(resumed.history)
        }
    }
}
