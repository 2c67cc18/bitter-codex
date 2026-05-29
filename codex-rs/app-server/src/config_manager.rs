use codex_arg0::Arg0DispatchPaths;
use codex_config::ConfigLayerStack;
use codex_config::LoaderOverrides;
use codex_config::ThreadConfigLoader;
use codex_config::loader::load_config_layers_state;
use codex_core::config::Config;
use codex_core::config::ConfigOverrides;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_json_to_toml::json_to_toml;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock;
use toml::Value as TomlValue;
use tracing::warn;

#[derive(Clone)]
pub(crate) struct ConfigManager {
    codex_home: PathBuf,
    cli_overrides: Arc<RwLock<Vec<(String, TomlValue)>>>,
    loader_overrides: LoaderOverrides,
    strict_config: bool,
    arg0_paths: Arg0DispatchPaths,
    thread_config_loader: Arc<RwLock<Arc<dyn ThreadConfigLoader>>>,
}

impl ConfigManager {
    pub(crate) fn new(
        codex_home: PathBuf,
        cli_overrides: Vec<(String, TomlValue)>,
        loader_overrides: LoaderOverrides,
        strict_config: bool,
        arg0_paths: Arg0DispatchPaths,
        thread_config_loader: Arc<dyn ThreadConfigLoader>,
    ) -> Self {
        Self {
            codex_home,
            cli_overrides: Arc::new(RwLock::new(cli_overrides)),
            loader_overrides,
            strict_config,
            arg0_paths,
            thread_config_loader: Arc::new(RwLock::new(thread_config_loader)),
        }
    }

    pub(crate) fn codex_home(&self) -> &Path {
        self.codex_home.as_path()
    }

    pub(crate) fn user_config_path(&self) -> std::io::Result<AbsolutePathBuf> {
        self.loader_overrides.user_config_path(self.codex_home())
    }

    pub(crate) fn current_cli_overrides(&self) -> Vec<(String, TomlValue)> {
        self.cli_overrides
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub(crate) fn replace_thread_config_loader(
        &self,
        thread_config_loader: Arc<dyn ThreadConfigLoader>,
    ) {
        if let Ok(mut guard) = self.thread_config_loader.write() {
            *guard = thread_config_loader;
        } else {
            warn!("failed to update thread config loader");
        }
    }

    fn current_thread_config_loader(&self) -> Arc<dyn ThreadConfigLoader> {
        self.thread_config_loader
            .read()
            .map(|guard| Arc::clone(&*guard))
            .unwrap_or_else(|_| Arc::new(codex_config::NoopThreadConfigLoader))
    }

    pub(crate) async fn load_latest_config(
        &self,
        fallback_cwd: Option<PathBuf>,
    ) -> std::io::Result<Config> {
        self.load_with_cli_overrides(
            &self.current_cli_overrides(),
            None,
            ConfigOverrides::default(),
            fallback_cwd,
        )
        .await
    }

    pub(crate) async fn load_default_config(&self) -> std::io::Result<Config> {
        let mut config = Config::load_default_with_cli_overrides_for_codex_home(
            self.codex_home.clone(),
            self.current_cli_overrides(),
        )
        .await?;
        if self.loader_overrides.user_config_path.is_some() {
            let user_config_path = self.loader_overrides.user_config_path(self.codex_home())?;
            config.config_layer_stack = config
                .config_layer_stack
                .with_user_config(&user_config_path, TomlValue::Table(toml::map::Map::new()));
        }
        self.apply_arg0_paths(&mut config);
        Ok(config)
    }

    pub(crate) async fn load_with_overrides(
        &self,
        request_overrides: Option<HashMap<String, serde_json::Value>>,
        typesafe_overrides: ConfigOverrides,
    ) -> std::io::Result<Config> {
        self.load_with_cli_overrides(
            &self.current_cli_overrides(),
            request_overrides,
            typesafe_overrides,
            None,
        )
        .await
    }

    pub(crate) async fn load_for_cwd(
        &self,
        request_overrides: Option<HashMap<String, serde_json::Value>>,
        typesafe_overrides: ConfigOverrides,
        cwd: Option<PathBuf>,
    ) -> std::io::Result<Config> {
        self.load_with_cli_overrides(
            &self.current_cli_overrides(),
            request_overrides,
            typesafe_overrides,
            cwd,
        )
        .await
    }

    pub(crate) async fn load_with_cli_overrides(
        &self,
        cli_overrides: &[(String, TomlValue)],
        request_overrides: Option<HashMap<String, serde_json::Value>>,
        typesafe_overrides: ConfigOverrides,
        fallback_cwd: Option<PathBuf>,
    ) -> std::io::Result<Config> {
        let merged_cli_overrides = cli_overrides
            .iter()
            .cloned()
            .chain(
                request_overrides
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(key, value)| (key, json_to_toml(value))),
            )
            .collect::<Vec<_>>();

        let mut config = codex_core::config::ConfigBuilder::default()
            .codex_home(self.codex_home.clone())
            .cli_overrides(merged_cli_overrides)
            .loader_overrides(self.loader_overrides.clone())
            .strict_config(self.strict_config)
            .harness_overrides(typesafe_overrides)
            .fallback_cwd(fallback_cwd)
            .thread_config_loader(self.current_thread_config_loader())
            .build()
            .await?;
        self.apply_arg0_paths(&mut config);
        Ok(config)
    }

    pub(crate) async fn load_config_layers(
        &self,
        cwd: Option<AbsolutePathBuf>,
    ) -> std::io::Result<ConfigLayerStack> {
        let thread_config_loader = self.current_thread_config_loader();
        load_config_layers_state(
            &self.codex_home,
            cwd,
            &self.current_cli_overrides(),
            codex_config::ConfigLoadOptions {
                loader_overrides: self.loader_overrides.clone(),
                strict_config: self.strict_config,
            },
            thread_config_loader.as_ref(),
        )
        .await
    }

    fn apply_arg0_paths(&self, config: &mut Config) {
        config.codex_self_exe = self.arg0_paths.codex_self_exe.clone();
        config.main_execve_wrapper_exe = self.arg0_paths.main_execve_wrapper_exe.clone();
    }
}
