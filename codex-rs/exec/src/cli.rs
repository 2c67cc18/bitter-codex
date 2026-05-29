use clap::Args;
use clap::FromArgMatches;
use clap::Parser;
use codex_utils_cli::CliConfigOverrides;
use codex_utils_cli::SharedCliOptions;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    version,
    override_usage = "bitter-codex exec [OPTIONS] [PROMPT]\n       bitter-codex exec [OPTIONS] <COMMAND> [ARGS]"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    #[arg(long = "strict-config", global = true, default_value_t = false)]
    pub strict_config: bool,

    #[clap(flatten)]
    pub shared: ExecSharedCliOptions,

    #[arg(long = "skip-git-repo-check", global = true, default_value_t = false)]
    pub skip_git_repo_check: bool,

    #[arg(long = "ephemeral", global = true, default_value_t = false)]
    pub ephemeral: bool,

    #[arg(long = "ignore-user-config", global = true, default_value_t = false)]
    pub ignore_user_config: bool,

    #[arg(long = "output-schema", value_name = "FILE", global = true)]
    pub output_schema: Option<PathBuf>,

    #[clap(skip)]
    pub config_overrides: CliConfigOverrides,

    #[arg(
        long = "json",
        default_value_t = false,
        global = true
    )]
    pub json: bool,

    #[arg(
        long = "output-last-message",
        short = 'o',
        value_name = "FILE",
        global = true
    )]
    pub last_message_file: Option<PathBuf>,

    #[arg(value_name = "PROMPT", value_hint = clap::ValueHint::Other)]
    pub prompt: Option<String>,
}

impl std::ops::Deref for Cli {
    type Target = SharedCliOptions;

    fn deref(&self) -> &Self::Target {
        &self.shared.0
    }
}

impl std::ops::DerefMut for Cli {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.shared.0
    }
}

#[derive(Debug, Default)]
pub struct ExecSharedCliOptions(SharedCliOptions);

impl ExecSharedCliOptions {
    pub fn into_inner(self) -> SharedCliOptions {
        self.0
    }
}

impl std::ops::Deref for ExecSharedCliOptions {
    type Target = SharedCliOptions;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for ExecSharedCliOptions {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Args for ExecSharedCliOptions {
    fn augment_args(cmd: clap::Command) -> clap::Command {
        mark_exec_global_args(SharedCliOptions::augment_args(cmd))
    }

    fn augment_args_for_update(cmd: clap::Command) -> clap::Command {
        mark_exec_global_args(SharedCliOptions::augment_args_for_update(cmd))
    }
}

impl FromArgMatches for ExecSharedCliOptions {
    fn from_arg_matches(matches: &clap::ArgMatches) -> Result<Self, clap::Error> {
        SharedCliOptions::from_arg_matches(matches).map(Self)
    }

    fn update_from_arg_matches(&mut self, matches: &clap::ArgMatches) -> Result<(), clap::Error> {
        self.0.update_from_arg_matches(matches)
    }
}

fn mark_exec_global_args(cmd: clap::Command) -> clap::Command {
    cmd.mut_arg("model", |arg| arg.global(true))
}

#[derive(Debug, clap::Subcommand)]
pub enum Command {
    Resume(ResumeArgs),
}

#[derive(Args, Debug)]
struct ResumeArgsRaw {
    #[arg(value_name = "SESSION_ID")]
    session_id: Option<String>,

    #[arg(long = "last", default_value_t = false)]
    last: bool,

    #[arg(long = "all", default_value_t = false)]
    all: bool,

    #[arg(
        long = "image",
        short = 'i',
        value_name = "FILE",
        value_delimiter = ',',
        num_args = 1
    )]
    images: Vec<PathBuf>,

    #[arg(value_name = "PROMPT", value_hint = clap::ValueHint::Other)]
    prompt: Option<String>,
}

#[derive(Debug)]
pub struct ResumeArgs {
    pub session_id: Option<String>,

    pub last: bool,

    pub all: bool,

    pub images: Vec<PathBuf>,

    pub prompt: Option<String>,
}

impl From<ResumeArgsRaw> for ResumeArgs {
    fn from(raw: ResumeArgsRaw) -> Self {
        let (session_id, prompt) = if raw.last && raw.prompt.is_none() {
            (None, raw.session_id)
        } else {
            (raw.session_id, raw.prompt)
        };
        Self {
            session_id,
            last: raw.last,
            all: raw.all,
            images: raw.images,
            prompt,
        }
    }
}

impl Args for ResumeArgs {
    fn augment_args(cmd: clap::Command) -> clap::Command {
        ResumeArgsRaw::augment_args(cmd)
    }

    fn augment_args_for_update(cmd: clap::Command) -> clap::Command {
        ResumeArgsRaw::augment_args_for_update(cmd)
    }
}

impl FromArgMatches for ResumeArgs {
    fn from_arg_matches(matches: &clap::ArgMatches) -> Result<Self, clap::Error> {
        ResumeArgsRaw::from_arg_matches(matches).map(Self::from)
    }

    fn update_from_arg_matches(&mut self, matches: &clap::ArgMatches) -> Result<(), clap::Error> {
        *self = ResumeArgsRaw::from_arg_matches(matches).map(Self::from)?;
        Ok(())
    }
}
