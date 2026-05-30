use std::path::PathBuf;

use clap::Parser;
use codex_app_server::AppServerRuntimeOptions;
use codex_app_server::AppServerTransport;
use codex_app_server::run_main_with_transport_options;
use codex_arg0::Arg0DispatchPaths;
use codex_arg0::arg0_dispatch_or_else;
use codex_config::LoaderOverrides;
use codex_protocol::protocol::SessionSource;
use codex_utils_cli::CliConfigOverrides;

// Debug-only test hook: lets integration tests point the server at a temporary
// managed config file without writing to /etc.
const MANAGED_CONFIG_PATH_ENV_VAR: &str = "CODEX_APP_SERVER_MANAGED_CONFIG_PATH";
const DISABLE_MANAGED_CONFIG_ENV_VAR: &str = "CODEX_APP_SERVER_DISABLE_MANAGED_CONFIG";

#[derive(Debug, Parser)]
#[command(version)]
struct AppServerArgs {
    /// Transport endpoint URL. Supported values: `unix://` (default), `unix://PATH`,
    /// `off`.
    #[arg(
        long = "listen",
        value_name = "URL",
        default_value = AppServerTransport::DEFAULT_LISTEN_URL
    )]
    listen: AppServerTransport,

    /// Session source used to derive product restrictions and metadata.
    #[arg(
        long = "session-source",
        value_name = "SOURCE",
        default_value = "vscode",
        value_parser = SessionSource::from_startup_arg
    )]
    session_source: SessionSource,

    /// Fail if config.toml contains unknown configuration fields.
    #[arg(long = "strict-config", default_value_t = false)]
    strict_config: bool,

    /// Hidden debug-only test hook retained for upstream harness compatibility.
    #[cfg(debug_assertions)]
    #[arg(long = "disable-plugin-startup-tasks-for-tests", hide = true)]
    disable_plugin_startup_tasks_for_tests: bool,
}

fn main() -> anyhow::Result<()> {
    arg0_dispatch_or_else(|arg0_paths: Arg0DispatchPaths| async move {
        let AppServerArgs {
            listen,
            session_source,
            strict_config,
            #[cfg(debug_assertions)]
                disable_plugin_startup_tasks_for_tests: _,
        } = AppServerArgs::parse();
        let loader_overrides = loader_overrides_from_debug_env();

        run_main_with_transport_options(
            arg0_paths,
            CliConfigOverrides::default(),
            loader_overrides,
            strict_config,
            listen,
            session_source,
            AppServerRuntimeOptions::default(),
        )
        .await?;
        Ok(())
    })
}

fn disable_managed_config_from_debug_env() -> bool {
    #[cfg(debug_assertions)]
    {
        if let Ok(value) = std::env::var(DISABLE_MANAGED_CONFIG_ENV_VAR) {
            return matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES");
        }
    }

    false
}

fn loader_overrides_from_debug_env() -> LoaderOverrides {
    if disable_managed_config_from_debug_env() {
        return LoaderOverrides::default();
    }

    managed_config_path_from_debug_env()
        .map(|system_config_path| LoaderOverrides {
            system_config_path: Some(system_config_path),
            ..Default::default()
        })
        .unwrap_or_default()
}

fn managed_config_path_from_debug_env() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    {
        if let Ok(value) = std::env::var(MANAGED_CONFIG_PATH_ENV_VAR) {
            return if value.is_empty() {
                None
            } else {
                Some(PathBuf::from(value))
            };
        }
    }

    None
}
