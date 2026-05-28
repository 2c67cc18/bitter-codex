use clap::Parser;
use codex_arg0::Arg0DispatchPaths;
use codex_arg0::arg0_dispatch_or_else;
use codex_cli::read_api_key_from_stdin;
use codex_cli::run_login_status;
use codex_cli::run_login_with_api_key;
use codex_cli::run_login_with_chatgpt;
use codex_cli::run_login_with_device_code;
use codex_cli::run_logout;
use codex_config::LoaderOverrides;
use codex_exec::Cli as ExecCli;
use codex_exec::Command as ExecCommand;
use codex_utils_cli::CliConfigOverrides;

#[derive(Debug, Parser)]
#[command(
    author,
    version,
    subcommand_negates_reqs = true,
    bin_name = "codex",
    override_usage = "codex [OPTIONS] [PROMPT]\n       codex [OPTIONS] <COMMAND> [ARGS]"
)]
struct MultitoolCli {
    #[clap(flatten)]
    config_overrides: CliConfigOverrides,

    #[command(subcommand)]
    subcommand: Option<Subcommand>,

    #[arg(value_name = "PROMPT", value_hint = clap::ValueHint::Other)]
    prompt: Option<String>,
}

#[derive(Debug, clap::Subcommand)]
enum Subcommand {
    #[clap(visible_alias = "e")]
    Exec(ExecCli),

    Login(LoginCommand),

    Logout(LogoutCommand),

    AppServer(AppServerCommand),
}

#[derive(Debug, Parser)]
struct LoginCommand {
    #[clap(skip)]
    config_overrides: CliConfigOverrides,

    #[arg(long = "with-api-key", default_value_t = false)]
    with_api_key: bool,

    #[arg(long = "api-key")]
    api_key: Option<String>,

    #[arg(long = "use-device-code", default_value_t = false)]
    use_device_code: bool,

    #[arg(long = "issuer-base-url")]
    issuer_base_url: Option<String>,

    #[arg(long = "client-id")]
    client_id: Option<String>,

    #[command(subcommand)]
    action: Option<LoginSubcommand>,
}

#[derive(Debug, clap::Subcommand)]
enum LoginSubcommand {
    Status,
}

#[derive(Debug, Parser)]
struct LogoutCommand {
    #[clap(skip)]
    config_overrides: CliConfigOverrides,
}

#[derive(Debug, Parser)]
struct AppServerCommand {
    #[arg(long = "strict-config", default_value_t = false)]
    strict_config: bool,

    #[arg(
        long = "listen",
        value_name = "URL",
        default_value = codex_app_server::AppServerTransport::DEFAULT_LISTEN_URL
    )]
    listen: codex_app_server::AppServerTransport,
}

fn main() -> anyhow::Result<()> {
    arg0_dispatch_or_else(|arg0_paths: Arg0DispatchPaths| async move { cli_main(arg0_paths).await })
}

async fn cli_main(arg0_paths: Arg0DispatchPaths) -> anyhow::Result<()> {
    let MultitoolCli {
        config_overrides,
        subcommand,
        prompt,
    } = MultitoolCli::parse();

    match subcommand {
        Some(Subcommand::Exec(mut exec_cli)) => {
            prepend_config_flags(&mut exec_cli.config_overrides, config_overrides);
            codex_exec::run_main(exec_cli, arg0_paths).await?;
        }
        Some(Subcommand::Login(mut login_cli)) => {
            prepend_config_flags(&mut login_cli.config_overrides, config_overrides);
            run_login_command(login_cli).await;
        }
        Some(Subcommand::Logout(mut logout_cli)) => {
            prepend_config_flags(&mut logout_cli.config_overrides, config_overrides);
            run_logout(logout_cli.config_overrides).await;
        }
        Some(Subcommand::AppServer(app_server_cli)) => {
            codex_app_server::run_main_with_transport_options(
                arg0_paths,
                config_overrides,
                LoaderOverrides::default(),
                app_server_cli.strict_config,
                app_server_cli.listen,
                codex_protocol::protocol::SessionSource::Exec,
                codex_app_server::AppServerRuntimeOptions::default(),
            )
            .await?;
        }
        None => {
            let exec_cli = ExecCli {
                command: None::<ExecCommand>,
                strict_config: false,
                shared: Default::default(),
                skip_git_repo_check: false,
                ephemeral: false,
                ignore_user_config: false,
                output_schema: None,
                config_overrides,
                json: false,
                last_message_file: None,
                prompt,
            };
            codex_exec::run_main(exec_cli, arg0_paths).await?;
        }
    }

    Ok(())
}

async fn run_login_command(login_cli: LoginCommand) {
    match login_cli.action {
        Some(LoginSubcommand::Status) => {
            run_login_status(login_cli.config_overrides).await;
        }
        None => {
            if login_cli.use_device_code {
                run_login_with_device_code(
                    login_cli.config_overrides,
                    login_cli.issuer_base_url,
                    login_cli.client_id,
                )
                .await;
            } else if login_cli.api_key.is_some() {
                eprintln!(
                    "The --api-key flag is no longer supported. Pipe the key instead, e.g. `printenv OPENAI_API_KEY | codex login --with-api-key`."
                );
                std::process::exit(1);
            } else if login_cli.with_api_key {
                let api_key = read_api_key_from_stdin();
                run_login_with_api_key(login_cli.config_overrides, api_key).await;
            } else {
                run_login_with_chatgpt(login_cli.config_overrides).await;
            }
        }
    }
}

fn prepend_config_flags(target: &mut CliConfigOverrides, root: CliConfigOverrides) {
    let existing = std::mem::take(&mut target.raw_overrides);
    target.raw_overrides = root.raw_overrides.into_iter().chain(existing).collect();
}
