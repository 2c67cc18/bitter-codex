use crate::path_utils::normalize_for_native_workdir;
use crate::unified_exec::DEFAULT_MAX_BACKGROUND_TERMINAL_TIMEOUT_MS;
use crate::unified_exec::MIN_EMPTY_YIELD_TIME_MS;
use codex_config::ConfigLayerSource;
use codex_config::ConfigLayerStack;
use codex_config::ConfigLayerStackOrdering;
use codex_config::ThreadConfigLoader;
use codex_config::config_toml::ConfigLockfileToml;
use codex_config::config_toml::ConfigToml;
use codex_config::config_toml::ProjectConfig;
use codex_config::config_toml::validate_model_providers;
use codex_config::loader::load_config_layers_state;
use codex_config::loader::project_trust_key;
use codex_config::types::AuthCredentialsStoreMode;
use codex_config::types::History;
use codex_config::types::Notice;
use codex_config::types::UriBasedFileOpener;
use codex_features::Feature;
use codex_features::FeatureConfigSource;
use codex_features::FeatureOverrides;
use codex_features::Features;
use codex_git_utils::resolve_root_git_project_for_trust;
use codex_login::AuthManagerConfig;
use codex_model_provider_info::ModelProviderInfo;
use codex_model_provider_info::built_in_model_providers;
use codex_model_provider_info::merge_configured_model_providers;
use codex_models_manager::ModelsManagerConfig;
use codex_protocol::config_types::AutoCompactTokenLimitScope;
use codex_protocol::config_types::ForcedLoginMethod;
use codex_protocol::config_types::ReasoningSummary;
use codex_protocol::config_types::SERVICE_TIER_DEFAULT_REQUEST_VALUE;
use codex_protocol::config_types::ServiceTier;
use codex_protocol::config_types::ShellEnvironmentPolicy;
use codex_protocol::config_types::TrustLevel;
use codex_protocol::config_types::Verbosity;
use codex_protocol::config_types::WebSearchConfig;
use codex_protocol::config_types::WebSearchMode;
use codex_protocol::openai_models::ModelsResponse;
use codex_protocol::openai_models::ReasoningEffort;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_absolute_path::AbsolutePathBufGuard;
use std::collections::HashMap;
use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use crate::config_lock::config_without_lock_controls;
use crate::config_lock::lock_layer_from_config;
use crate::config_lock::read_config_lock_from_path;
use toml::Value as TomlValue;
use toml_edit::DocumentMut;

pub mod edit;
mod managed_features;
mod otel;
pub use codex_config::ConfigLoadOptions;
pub use codex_config::Constrained;
pub use codex_config::ConstraintError;
pub use codex_config::ConstraintResult;
pub use codex_config::LoaderOverrides;
pub use managed_features::ManagedFeatures;

const DEFAULT_IGNORE_LARGE_UNTRACKED_DIRS: i64 = 200;
const DEFAULT_IGNORE_LARGE_UNTRACKED_FILES: i64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhostSnapshotConfig {
    pub ignore_large_untracked_files: Option<i64>,
    pub ignore_large_untracked_dirs: Option<i64>,
    pub disable_warnings: bool,
}

impl Default for GhostSnapshotConfig {
    fn default() -> Self {
        Self {
            ignore_large_untracked_files: Some(DEFAULT_IGNORE_LARGE_UNTRACKED_FILES),
            ignore_large_untracked_dirs: Some(DEFAULT_IGNORE_LARGE_UNTRACKED_DIRS),
            disable_warnings: false,
        }
    }
}

const LOCAL_DEV_BUILD_VERSION: &str = "0.0.0";

pub const CONFIG_TOML_FILE: &str = "config.toml";

fn resolve_sqlite_home_env(resolved_cwd: &Path) -> Option<PathBuf> {
    let raw = std::env::var(codex_state::SQLITE_HOME_ENV).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = PathBuf::from(trimmed);
    if path.is_absolute() {
        Some(path)
    } else {
        Some(resolved_cwd.join(path))
    }
}

fn resolve_cli_auth_credentials_store_mode(
    configured: AuthCredentialsStoreMode,
    package_version: &str,
) -> AuthCredentialsStoreMode {
    match (package_version, configured) {
        (
            LOCAL_DEV_BUILD_VERSION,
            AuthCredentialsStoreMode::Keyring | AuthCredentialsStoreMode::Auto,
        ) => AuthCredentialsStoreMode::File,
        (_, mode) => mode,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub config_layer_stack: ConfigLayerStack,

    pub startup_warnings: Vec<String>,

    pub model: Option<String>,

    pub service_tier: Option<String>,

    pub model_context_window: Option<i64>,

    pub model_auto_compact_token_limit: Option<i64>,

    pub model_auto_compact_token_limit_scope: AutoCompactTokenLimitScope,

    pub model_provider_id: String,

    pub model_provider: ModelProviderInfo,

    pub hide_agent_reasoning: bool,

    pub show_raw_agent_reasoning: bool,

    pub base_instructions: Option<String>,

    pub developer_instructions: Option<String>,

    pub include_environment_context: bool,

    pub compact_prompt: Option<String>,

    pub notify: Option<Vec<String>>,

    pub cwd: AbsolutePathBuf,

    pub workspace_roots: Vec<AbsolutePathBuf>,

    pub workspace_roots_explicit: bool,

    pub allow_login_shell: bool,

    pub shell_environment_policy: ShellEnvironmentPolicy,

    pub cli_auth_credentials_store_mode: AuthCredentialsStoreMode,

    pub model_providers: HashMap<String, ModelProviderInfo>,

    pub tool_output_token_limit: Option<usize>,
    pub codex_home: AbsolutePathBuf,

    pub sqlite_home: PathBuf,

    pub log_dir: PathBuf,

    pub config_lock_export_dir: Option<AbsolutePathBuf>,

    pub config_lock_allow_codex_version_mismatch: bool,

    pub config_lock_save_fields_resolved_from_model_catalog: bool,

    pub config_lock_toml: Option<Arc<ConfigLockfileToml>>,

    pub history: History,

    pub ephemeral: bool,

    pub file_opener: UriBasedFileOpener,

    pub codex_self_exe: Option<PathBuf>,

    pub main_execve_wrapper_exe: Option<PathBuf>,

    pub zsh_path: Option<PathBuf>,

    pub model_reasoning_effort: Option<ReasoningEffort>,

    pub plan_mode_reasoning_effort: Option<ReasoningEffort>,

    pub model_reasoning_summary: Option<ReasoningSummary>,

    pub model_supports_reasoning_summaries: Option<bool>,

    pub model_catalog: Option<ModelsResponse>,

    pub model_verbosity: Option<Verbosity>,

    pub chatgpt_base_url: String,

    pub forced_chatgpt_workspace_id: Option<Vec<String>>,

    pub forced_login_method: Option<ForcedLoginMethod>,

    pub web_search_mode: Constrained<WebSearchMode>,

    pub web_search_config: Option<WebSearchConfig>,

    pub background_terminal_max_timeout: u64,

    pub ghost_snapshot: GhostSnapshotConfig,

    pub features: ManagedFeatures,

    pub active_project: ProjectConfig,

    pub notices: Notice,

    pub check_for_update_on_startup: bool,

    pub disable_paste_burst: bool,

    pub otel: codex_config::types::OtelConfig,
}

impl AuthManagerConfig for Config {
    fn codex_home(&self) -> PathBuf {
        self.codex_home.to_path_buf()
    }

    fn cli_auth_credentials_store_mode(&self) -> AuthCredentialsStoreMode {
        self.cli_auth_credentials_store_mode
    }

    fn forced_chatgpt_workspace_id(&self) -> Option<Vec<String>> {
        self.forced_chatgpt_workspace_id.clone()
    }

    fn chatgpt_base_url(&self) -> String {
        self.chatgpt_base_url.clone()
    }
}

#[derive(Clone, Default)]
pub struct ConfigBuilder {
    codex_home: Option<PathBuf>,
    cli_overrides: Option<Vec<(String, TomlValue)>>,
    harness_overrides: Option<ConfigOverrides>,
    loader_overrides: Option<LoaderOverrides>,
    strict_config: bool,
    thread_config_loader: Option<Arc<dyn ThreadConfigLoader>>,
    fallback_cwd: Option<PathBuf>,
}

impl ConfigBuilder {
    pub fn codex_home(mut self, codex_home: PathBuf) -> Self {
        self.codex_home = Some(codex_home);
        self
    }

    pub fn cli_overrides(mut self, cli_overrides: Vec<(String, TomlValue)>) -> Self {
        self.cli_overrides = Some(cli_overrides);
        self
    }

    pub fn harness_overrides(mut self, harness_overrides: ConfigOverrides) -> Self {
        self.harness_overrides = Some(harness_overrides);
        self
    }

    pub fn loader_overrides(mut self, loader_overrides: LoaderOverrides) -> Self {
        self.loader_overrides = Some(loader_overrides);
        self
    }

    pub fn strict_config(mut self, strict_config: bool) -> Self {
        self.strict_config = strict_config;
        self
    }

    pub fn thread_config_loader(
        mut self,
        thread_config_loader: Arc<dyn ThreadConfigLoader>,
    ) -> Self {
        self.thread_config_loader = Some(thread_config_loader);
        self
    }

    pub fn fallback_cwd(mut self, fallback_cwd: Option<PathBuf>) -> Self {
        self.fallback_cwd = fallback_cwd;
        self
    }

    pub async fn build(self) -> std::io::Result<Config> {
        Box::pin(self.build_inner()).await
    }

    async fn build_inner(self) -> std::io::Result<Config> {
        let Self {
            codex_home,
            cli_overrides,
            harness_overrides,
            loader_overrides,
            strict_config,
            thread_config_loader,
            fallback_cwd,
        } = self;
        let codex_home = match codex_home {
            Some(codex_home) => AbsolutePathBuf::from_absolute_path(codex_home)?,
            None => find_codex_home()?,
        };
        let cli_overrides = cli_overrides.unwrap_or_default();
        let mut harness_overrides = harness_overrides.unwrap_or_default();
        let loader_overrides = loader_overrides.unwrap_or_default();
        let cwd_override = harness_overrides.cwd.as_deref().or(fallback_cwd.as_deref());
        let cwd = match cwd_override {
            Some(path) => AbsolutePathBuf::relative_to_current_dir(path)?,
            None => AbsolutePathBuf::current_dir()?,
        };
        harness_overrides.cwd = Some(cwd.to_path_buf());
        let config_layer_stack = load_config_layers_state(
            &codex_home,
            Some(cwd),
            &cli_overrides,
            ConfigLoadOptions {
                loader_overrides,
                strict_config,
            },
            thread_config_loader
                .as_deref()
                .unwrap_or(&codex_config::NoopThreadConfigLoader),
        )
        .await?;
        let merged_toml = config_layer_stack.effective_config();

        let config_toml: ConfigToml = match merged_toml.try_into() {
            Ok(config_toml) => config_toml,
            Err(err) => {
                if let Some(config_error) = codex_config::first_layer_config_error::<ConfigToml>(
                    &config_layer_stack,
                    codex_config::CONFIG_TOML_FILE,
                )
                .await
                {
                    return Err(codex_config::io_error_from_config_error(
                        std::io::ErrorKind::InvalidData,
                        config_error,
                        Some(err),
                    ));
                }
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, err));
            }
        };
        let config_lock_settings = config_toml
            .debug
            .as_ref()
            .and_then(|debug| debug.config_lockfile.as_ref());
        if let Some(config_lock_load_path) =
            config_lock_settings.and_then(|config_lock| config_lock.load_path.as_ref())
        {
            let allow_codex_version_mismatch = config_lock_settings
                .and_then(|config_lock| config_lock.allow_codex_version_mismatch)
                .unwrap_or(false);
            let save_fields_resolved_from_model_catalog = config_lock_settings
                .and_then(|config_lock| config_lock.save_fields_resolved_from_model_catalog)
                .unwrap_or(true);
            let lockfile_toml = read_config_lock_from_path(config_lock_load_path).await?;
            let expected_lock_config = lockfile_toml.clone();
            let lock_layer = lock_layer_from_config(config_lock_load_path, &lockfile_toml)?;
            let lock_config_toml = config_without_lock_controls(&lockfile_toml.config);
            let lock_config_layer_stack = ConfigLayerStack::new(vec![lock_layer])?;
            let mut config = Config::load_config_with_layer_stack(
                lock_config_toml,
                harness_overrides,
                codex_home,
                lock_config_layer_stack,
            )
            .await?;
            config.config_lock_toml = Some(Arc::new(expected_lock_config));
            config.config_lock_allow_codex_version_mismatch = allow_codex_version_mismatch;
            config.config_lock_save_fields_resolved_from_model_catalog =
                save_fields_resolved_from_model_catalog;
            return Ok(config);
        }
        Config::load_config_with_layer_stack(
            config_toml,
            harness_overrides,
            codex_home,
            config_layer_stack,
        )
        .await
    }
}

impl Config {
    pub fn effective_workspace_roots(&self) -> Vec<AbsolutePathBuf> {
        let mut workspace_roots = self.workspace_roots.clone();
        dedupe_absolute_paths(&mut workspace_roots);
        workspace_roots
    }

    pub fn to_models_manager_config(&self) -> ModelsManagerConfig {
        ModelsManagerConfig {
            model_context_window: self.model_context_window,
            model_auto_compact_token_limit: self.model_auto_compact_token_limit,
            tool_output_token_limit: self.tool_output_token_limit,
            base_instructions: self.base_instructions.clone(),
            model_supports_reasoning_summaries: self.model_supports_reasoning_summaries,
            model_catalog: self.model_catalog.clone(),
        }
    }

    pub async fn rebuild_preserving_session_layers(
        &self,
        refreshed_config: &Config,
    ) -> std::io::Result<Self> {
        let mut layers = refreshed_config
            .config_layer_stack
            .get_layers(ConfigLayerStackOrdering::LowestPrecedenceFirst, true)
            .into_iter()
            .filter(|layer| !is_session_layer(&layer.name))
            .cloned()
            .collect::<Vec<_>>();
        layers.extend(
            self.config_layer_stack
                .get_layers(ConfigLayerStackOrdering::LowestPrecedenceFirst, true)
                .into_iter()
                .filter(|layer| is_session_layer(&layer.name))
                .cloned(),
        );
        layers.sort_by_key(|layer| layer.name.precedence());

        let config_layer_stack = ConfigLayerStack::new(layers)?;
        let cfg: ConfigToml = config_layer_stack
            .effective_config()
            .try_into()
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
        Self::load_config_with_layer_stack(
            cfg,
            ConfigOverrides {
                cwd: Some(self.cwd.to_path_buf()),
                ..Default::default()
            },
            refreshed_config.codex_home.clone(),
            config_layer_stack,
        )
        .await
    }

    pub async fn load_with_cli_overrides(
        cli_overrides: Vec<(String, TomlValue)>,
    ) -> std::io::Result<Self> {
        ConfigBuilder::default()
            .cli_overrides(cli_overrides)
            .build()
            .await
    }

    pub async fn load_default_with_cli_overrides(
        cli_overrides: Vec<(String, TomlValue)>,
    ) -> std::io::Result<Self> {
        let codex_home = find_codex_home()?;
        Self::load_default_with_cli_overrides_for_codex_home(
            codex_home.to_path_buf(),
            cli_overrides,
        )
        .await
    }

    pub async fn load_default_with_cli_overrides_for_codex_home(
        codex_home: PathBuf,
        cli_overrides: Vec<(String, TomlValue)>,
    ) -> std::io::Result<Self> {
        let mut merged = toml::Value::try_from(ConfigToml::default()).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("failed to serialize default config: {e}"),
            )
        })?;
        let cli_layer = codex_config::build_cli_overrides_layer(&cli_overrides);
        codex_config::merge_toml_values(&mut merged, &cli_layer);
        let codex_home = AbsolutePathBuf::from_absolute_path_checked(codex_home)?;
        let config_toml = deserialize_config_toml_with_base(merged, &codex_home)?;
        Self::load_config_with_layer_stack(
            config_toml,
            ConfigOverrides::default(),
            codex_home,
            ConfigLayerStack::default(),
        )
        .await
    }

    pub async fn load_with_cli_overrides_and_harness_overrides(
        cli_overrides: Vec<(String, TomlValue)>,
        harness_overrides: ConfigOverrides,
    ) -> std::io::Result<Self> {
        ConfigBuilder::default()
            .cli_overrides(cli_overrides)
            .harness_overrides(harness_overrides)
            .build()
            .await
    }
}

pub async fn load_config_as_toml_with_cli_overrides(
    codex_home: &Path,
    cwd: Option<&AbsolutePathBuf>,
    cli_overrides: Vec<(String, TomlValue)>,
    loader_overrides: LoaderOverrides,
) -> std::io::Result<ConfigToml> {
    load_config_as_toml_with_cli_and_loader_overrides(
        codex_home,
        cwd,
        cli_overrides,
        loader_overrides,
    )
    .await
}

pub async fn load_config_as_toml_with_cli_and_loader_overrides(
    codex_home: &Path,
    cwd: Option<&AbsolutePathBuf>,
    cli_overrides: Vec<(String, TomlValue)>,
    loader_overrides: LoaderOverrides,
) -> std::io::Result<ConfigToml> {
    load_config_as_toml_with_cli_and_load_options(codex_home, cwd, cli_overrides, loader_overrides)
        .await
}

pub async fn load_config_as_toml_with_cli_and_load_options(
    codex_home: &Path,
    cwd: Option<&AbsolutePathBuf>,
    cli_overrides: Vec<(String, TomlValue)>,
    options: impl Into<ConfigLoadOptions>,
) -> std::io::Result<ConfigToml> {
    let config_layer_stack = load_config_layers_state(
        codex_home,
        cwd.cloned(),
        &cli_overrides,
        options,
        &codex_config::NoopThreadConfigLoader,
    )
    .await?;

    let merged_toml = config_layer_stack.effective_config();
    let cfg = deserialize_config_toml_with_base(merged_toml, codex_home).map_err(|e| {
        tracing::error!("Failed to deserialize overridden config: {e}");
        e
    })?;

    Ok(cfg)
}

pub fn deserialize_config_toml_with_base(
    root_value: TomlValue,
    config_base_dir: &Path,
) -> std::io::Result<ConfigToml> {
    let _guard = AbsolutePathBufGuard::new(config_base_dir);
    root_value
        .try_into()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

fn load_catalog_json(path: &AbsolutePathBuf) -> std::io::Result<ModelsResponse> {
    let file_contents = std::fs::read_to_string(path)?;
    let catalog = serde_json::from_str::<ModelsResponse>(&file_contents).map_err(|err| {
        std::io::Error::new(
            ErrorKind::InvalidData,
            format!(
                "failed to parse model_catalog_json path `{}` as JSON: {err}",
                path.display()
            ),
        )
    })?;
    if catalog.models.is_empty() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            format!(
                "model_catalog_json path `{}` must contain at least one model",
                path.display()
            ),
        ));
    }
    Ok(catalog)
}

fn load_model_catalog(
    model_catalog_json: Option<AbsolutePathBuf>,
) -> std::io::Result<Option<ModelsResponse>> {
    model_catalog_json
        .map(|path| load_catalog_json(&path))
        .transpose()
}
pub(crate) fn set_project_trust_level_inner(
    doc: &mut DocumentMut,
    project_path: &Path,
    trust_level: TrustLevel,
) -> anyhow::Result<()> {
    let project_key = project_trust_key(project_path);

    {
        let root = doc.as_table_mut();

        let existing_projects = root.get("projects").cloned();
        if existing_projects.as_ref().is_none_or(|i| !i.is_table()) {
            let mut projects_tbl = toml_edit::Table::new();
            projects_tbl.set_implicit(true);

            if let Some(inline_tbl) = existing_projects.as_ref().and_then(|i| i.as_inline_table()) {
                for (k, v) in inline_tbl.iter() {
                    if let Some(inner_tbl) = v.as_inline_table() {
                        let new_tbl = inner_tbl.clone().into_table();
                        projects_tbl.insert(k, toml_edit::Item::Table(new_tbl));
                    }
                }
            }

            root.insert("projects", toml_edit::Item::Table(projects_tbl));
        }
    }
    let Some(projects_tbl) = doc["projects"].as_table_mut() else {
        return Err(anyhow::anyhow!(
            "projects table missing after initialization"
        ));
    };

    let needs_proj_table = !projects_tbl.contains_key(project_key.as_str())
        || projects_tbl
            .get(project_key.as_str())
            .and_then(|i| i.as_table())
            .is_none();
    if needs_proj_table {
        projects_tbl.insert(project_key.as_str(), toml_edit::table());
    }
    let Some(proj_tbl) = projects_tbl
        .get_mut(project_key.as_str())
        .and_then(|i| i.as_table_mut())
    else {
        return Err(anyhow::anyhow!("project table missing for {project_key}"));
    };
    proj_tbl.set_implicit(false);
    proj_tbl["trust_level"] = toml_edit::value(trust_level.to_string());
    Ok(())
}

pub fn set_project_trust_level(
    codex_home: &Path,
    project_path: &Path,
    trust_level: TrustLevel,
) -> anyhow::Result<()> {
    use crate::config::edit::ConfigEditsBuilder;

    ConfigEditsBuilder::new(codex_home)
        .set_project_trust_level(project_path, trust_level)
        .apply_blocking()
}

fn is_session_layer(source: &ConfigLayerSource) -> bool {
    matches!(source, ConfigLayerSource::SessionFlags)
}

#[derive(Default, Debug, Clone)]
pub struct ConfigOverrides {
    pub model: Option<String>,
    pub cwd: Option<PathBuf>,
    pub model_provider: Option<String>,
    pub service_tier: Option<Option<String>>,
    pub codex_self_exe: Option<PathBuf>,
    pub main_execve_wrapper_exe: Option<PathBuf>,
    pub zsh_path: Option<PathBuf>,
    pub base_instructions: Option<String>,
    pub developer_instructions: Option<String>,
    pub compact_prompt: Option<String>,
    pub show_raw_agent_reasoning: Option<bool>,
    pub tools_web_search_request: Option<bool>,
    pub ephemeral: Option<bool>,

    pub additional_writable_roots: Vec<PathBuf>,

    pub workspace_roots: Option<Vec<PathBuf>>,
}

fn dedupe_absolute_paths(paths: &mut Vec<AbsolutePathBuf>) {
    let mut seen = HashSet::new();
    paths.retain(|path| seen.insert(path.clone()));
}

fn resolve_web_search_mode(config_toml: &ConfigToml, features: &Features) -> Option<WebSearchMode> {
    if let Some(mode) = config_toml.web_search {
        return Some(mode);
    }
    if features.enabled(Feature::WebSearchCached) {
        return Some(WebSearchMode::Cached);
    }
    if features.enabled(Feature::WebSearchRequest) {
        return Some(WebSearchMode::Live);
    }
    None
}

fn resolve_web_search_config(config_toml: &ConfigToml) -> Option<WebSearchConfig> {
    config_toml
        .tools
        .as_ref()
        .and_then(|tools| tools.web_search.as_ref())
        .cloned()
        .map(Into::into)
}

impl Config {
    pub(crate) async fn load_config_with_layer_stack(
        cfg: ConfigToml,
        overrides: ConfigOverrides,
        codex_home: AbsolutePathBuf,
        config_layer_stack: ConfigLayerStack,
    ) -> std::io::Result<Self> {
        validate_model_providers(&cfg.model_providers)
            .map_err(|message| std::io::Error::new(std::io::ErrorKind::InvalidInput, message))?;
        let mut startup_warnings = config_layer_stack
            .startup_warnings()
            .unwrap_or_default()
            .to_vec();

        let ConfigOverrides {
            model,
            cwd,
            model_provider,
            service_tier: service_tier_override,
            codex_self_exe,
            main_execve_wrapper_exe,
            zsh_path: zsh_path_override,
            base_instructions,
            developer_instructions,
            compact_prompt,
            show_raw_agent_reasoning,
            tools_web_search_request: override_tools_web_search_request,
            ephemeral,
            additional_writable_roots,
            workspace_roots: workspace_roots_override,
        } = overrides;
        let feature_overrides = FeatureOverrides {
            web_search_request: override_tools_web_search_request,
        };

        let configured_features = Features::from_sources(
            FeatureConfigSource {
                features: cfg.features.as_ref(),
            },
            feature_overrides,
        );
        let features = ManagedFeatures::from_configured_with_warnings(
            configured_features,
            &mut startup_warnings,
        )?;
        let resolved_cwd = AbsolutePathBuf::try_from(normalize_for_native_workdir({
            use std::env;

            match cwd {
                None => {
                    tracing::info!("cwd not set, using current dir");
                    env::current_dir()?
                }
                Some(p) if p.is_absolute() => p,
                Some(p) => {
                    tracing::info!("cwd is relative, resolving against current dir");
                    let mut current = env::current_dir()?;
                    current.push(p);
                    current
                }
            }
        }))?;
        let requested_additional_writable_roots: Vec<AbsolutePathBuf> = additional_writable_roots
            .into_iter()
            .map(|path| AbsolutePathBuf::resolve_path_against_base(path, resolved_cwd.as_path()))
            .collect();
        let repo_root = resolve_root_git_project_for_trust(&resolved_cwd);
        let active_project = cfg
            .get_active_project(
                resolved_cwd.as_path(),
                repo_root.as_ref().map(AbsolutePathBuf::as_path),
            )
            .unwrap_or(ProjectConfig { trust_level: None });
        let workspace_roots_explicit = workspace_roots_override.is_some()
            || !requested_additional_writable_roots.is_empty()
            || false;
        let mut workspace_roots = match workspace_roots_override {
            Some(workspace_roots) => workspace_roots
                .into_iter()
                .map(|path| {
                    AbsolutePathBuf::resolve_path_against_base(path, resolved_cwd.as_path())
                })
                .collect(),
            None => {
                let mut workspace_roots = vec![resolved_cwd.clone()];
                workspace_roots.extend(requested_additional_writable_roots.clone());
                workspace_roots
            }
        };
        dedupe_absolute_paths(&mut workspace_roots);
        let web_search_mode =
            resolve_web_search_mode(&cfg, &features).unwrap_or(WebSearchMode::Cached);
        let web_search_config = resolve_web_search_config(&cfg);

        let openai_base_url = cfg
            .openai_base_url
            .clone()
            .filter(|value| !value.is_empty());

        let model_providers = merge_configured_model_providers(
            built_in_model_providers(openai_base_url),
            cfg.model_providers,
        )
        .map_err(|message| std::io::Error::new(std::io::ErrorKind::InvalidData, message))?;

        let model_provider_id = model_provider
            .or(cfg.model_provider)
            .unwrap_or_else(|| "openai".to_string());
        let model_provider = model_providers
            .get(&model_provider_id)
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("Model provider `{model_provider_id}` not found"),
                )
            })?
            .clone();

        let shell_environment_policy = cfg.shell_environment_policy.into();
        let allow_login_shell = cfg.allow_login_shell.unwrap_or(true);

        let history = cfg.history.unwrap_or_default();
        let background_terminal_max_timeout = cfg
            .background_terminal_max_timeout
            .unwrap_or(DEFAULT_MAX_BACKGROUND_TERMINAL_TIMEOUT_MS)
            .max(MIN_EMPTY_YIELD_TIME_MS);

        let ghost_snapshot = {
            let mut config = GhostSnapshotConfig::default();
            if let Some(ghost_snapshot) = cfg.ghost_snapshot.as_ref()
                && let Some(ignore_over_bytes) = ghost_snapshot.ignore_large_untracked_files
            {
                config.ignore_large_untracked_files = if ignore_over_bytes > 0 {
                    Some(ignore_over_bytes)
                } else {
                    None
                };
            }
            if let Some(ghost_snapshot) = cfg.ghost_snapshot.as_ref()
                && let Some(threshold) = ghost_snapshot.ignore_large_untracked_dirs
            {
                config.ignore_large_untracked_dirs =
                    if threshold > 0 { Some(threshold) } else { None };
            }
            if let Some(ghost_snapshot) = cfg.ghost_snapshot.as_ref()
                && let Some(disable_warnings) = ghost_snapshot.disable_warnings
            {
                config.disable_warnings = disable_warnings;
            }
            config
        };

        let forced_chatgpt_workspace_id = cfg
            .forced_chatgpt_workspace_id
            .clone()
            .map(codex_config::config_toml::ForcedChatgptWorkspaceIds::into_vec)
            .map(|values| {
                values
                    .into_iter()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .collect::<Vec<_>>()
            })
            .filter(|values| !values.is_empty());

        let forced_login_method = cfg.forced_login_method;

        let model = model.or(cfg.model);
        let notices = cfg.notice.unwrap_or_default();
        let service_tier = match service_tier_override {
            Some(Some(service_tier)) => Some(service_tier),
            Some(None) => Some(SERVICE_TIER_DEFAULT_REQUEST_VALUE.to_string()),
            None => cfg.service_tier,
        };
        let service_tier = service_tier.and_then(|service_tier| {
            match ServiceTier::from_request_value(&service_tier) {
                Some(ServiceTier::Fast) => Some(ServiceTier::Fast.request_value().to_string()),
                Some(ServiceTier::Flex) => Some(ServiceTier::Flex.request_value().to_string()),
                None => Some(service_tier),
            }
        });

        let compact_prompt = compact_prompt.or(cfg.compact_prompt).and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });

        let model_instructions_path = cfg.model_instructions_file.as_ref();
        let file_base_instructions =
            read_non_empty_config_file(model_instructions_path, "model instructions file")?;
        let base_instructions = base_instructions
            .or(file_base_instructions)
            .or(cfg.instructions.clone());
        let developer_instructions = developer_instructions.or(cfg.developer_instructions);
        let include_environment_context = cfg.include_environment_context.unwrap_or(true);

        let zsh_path = zsh_path_override.or(cfg.zsh_path.map(Into::into));

        let check_for_update_on_startup = cfg.check_for_update_on_startup.unwrap_or(true);
        let model_catalog = load_model_catalog(cfg.model_catalog_json.clone())?;

        let log_dir = cfg
            .log_dir
            .as_ref()
            .map(AbsolutePathBuf::to_path_buf)
            .unwrap_or_else(|| codex_home.join("log").to_path_buf());
        let sqlite_home = cfg
            .sqlite_home
            .as_ref()
            .map(AbsolutePathBuf::to_path_buf)
            .or_else(|| resolve_sqlite_home_env(&resolved_cwd))
            .unwrap_or_else(|| codex_home.to_path_buf());
        let otel = otel::resolve_config(cfg.otel.unwrap_or_default(), &mut startup_warnings);
        let config = Self {
            config_layer_stack,
            startup_warnings,
            model,
            service_tier,
            model_context_window: cfg.model_context_window,
            model_auto_compact_token_limit: cfg.model_auto_compact_token_limit,
            model_auto_compact_token_limit_scope: cfg
                .model_auto_compact_token_limit_scope
                .unwrap_or_default(),
            model_provider_id,
            model_provider,
            hide_agent_reasoning: cfg.hide_agent_reasoning.unwrap_or(false),
            show_raw_agent_reasoning: cfg
                .show_raw_agent_reasoning
                .or(show_raw_agent_reasoning)
                .unwrap_or(false),
            base_instructions,
            developer_instructions,
            include_environment_context,
            compact_prompt,
            notify: cfg.notify,
            cwd: resolved_cwd,
            workspace_roots,
            workspace_roots_explicit,
            allow_login_shell,
            shell_environment_policy,
            cli_auth_credentials_store_mode: resolve_cli_auth_credentials_store_mode(
                cfg.cli_auth_credentials_store.unwrap_or_default(),
                env!("CARGO_PKG_VERSION"),
            ),
            model_providers,
            tool_output_token_limit: cfg.tool_output_token_limit,
            codex_home,
            sqlite_home,
            log_dir,
            config_lock_export_dir: cfg
                .debug
                .as_ref()
                .and_then(|debug| debug.config_lockfile.as_ref())
                .and_then(|config_lock| config_lock.export_dir.clone()),
            config_lock_allow_codex_version_mismatch: cfg
                .debug
                .as_ref()
                .and_then(|debug| debug.config_lockfile.as_ref())
                .and_then(|config_lock| config_lock.allow_codex_version_mismatch)
                .unwrap_or(false),
            config_lock_save_fields_resolved_from_model_catalog: cfg
                .debug
                .as_ref()
                .and_then(|debug| debug.config_lockfile.as_ref())
                .and_then(|config_lock| config_lock.save_fields_resolved_from_model_catalog)
                .unwrap_or(true),
            config_lock_toml: None,
            history,
            ephemeral: ephemeral.unwrap_or_default(),
            file_opener: cfg.file_opener.unwrap_or(UriBasedFileOpener::VsCode),
            codex_self_exe,
            main_execve_wrapper_exe,
            zsh_path,
            model_reasoning_effort: cfg.model_reasoning_effort,
            plan_mode_reasoning_effort: cfg.plan_mode_reasoning_effort,
            model_reasoning_summary: cfg.model_reasoning_summary,
            model_supports_reasoning_summaries: cfg.model_supports_reasoning_summaries,
            model_catalog,
            model_verbosity: cfg.model_verbosity,
            chatgpt_base_url: cfg
                .chatgpt_base_url
                .unwrap_or("https://chatgpt.com/backend-api/".to_string()),
            forced_chatgpt_workspace_id,
            forced_login_method,
            web_search_mode: Constrained::allow_any(web_search_mode),
            web_search_config,
            background_terminal_max_timeout,
            ghost_snapshot,
            features,
            active_project,
            notices,
            check_for_update_on_startup,
            disable_paste_burst: cfg.disable_paste_burst.unwrap_or(false),
            otel,
        };
        Ok(config)
    }
}

fn read_non_empty_config_file<P: AsRef<std::path::Path>>(
    path: Option<&P>,
    description: &str,
) -> std::io::Result<Option<String>> {
    let Some(path) = path else {
        return Ok(None);
    };
    let path = path.as_ref();
    let contents = std::fs::read_to_string(path).map_err(|err| {
        std::io::Error::new(
            err.kind(),
            format!("failed to read {description} `{}`: {err}", path.display()),
        )
    })?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

pub fn find_codex_home() -> std::io::Result<AbsolutePathBuf> {
    codex_utils_home_dir::find_codex_home()
}

pub fn log_dir(cfg: &Config) -> std::io::Result<PathBuf> {
    Ok(cfg.log_dir.clone())
}
