#![expect(clippy::expect_used)]

use anyhow::Context as _;
use anyhow::ensure;
use codex_arg0::Arg0PathEntryGuard;
use codex_utils_cargo_bin::CargoBinError;
use ctor::ctor;
use std::sync::OnceLock;
use tempfile::TempDir;

use codex_config::LoaderOverrides;
use codex_core::CodexThread;
use codex_core::config::Config;
use codex_core::config::ConfigBuilder;
use codex_core::config::ConfigOverrides;
use codex_utils_absolute_path::AbsolutePathBuf;
pub use codex_utils_absolute_path::test_support::PathBufExt;
pub use codex_utils_absolute_path::test_support::PathExt;
use regex_lite::Regex;
use std::path::PathBuf;

pub mod process;
pub mod responses;
pub mod streaming_sse;
pub mod test_codex_exec;
pub mod tracing;

static TEST_ARG0_PATH_ENTRY: OnceLock<Option<Arg0PathEntryGuard>> = OnceLock::new();

#[ctor]
fn enable_deterministic_unified_exec_process_ids_for_tests() {
    codex_core::test_support::set_thread_manager_test_mode(true);
    codex_core::test_support::set_deterministic_process_ids(true);
}

#[ctor]
fn configure_arg0_dispatch_for_test_binaries() {
    let _ = TEST_ARG0_PATH_ENTRY.get_or_init(codex_arg0::arg0_dispatch);
}

#[ctor]
fn configure_insta_workspace_root_for_snapshot_tests() {
    if std::env::var_os("INSTA_WORKSPACE_ROOT").is_some() {
        return;
    }

    let workspace_root = codex_utils_cargo_bin::repo_root()
        .ok()
        .map(|root| root.join("codex-rs"));

    if let Some(workspace_root) = workspace_root
        && let Ok(workspace_root) = workspace_root.canonicalize()
    {
        unsafe {
            std::env::set_var("INSTA_WORKSPACE_ROOT", workspace_root);
        }
    }
}

#[track_caller]
pub fn assert_regex_match<'s>(pattern: &str, actual: &'s str) -> regex_lite::Captures<'s> {
    let regex = Regex::new(pattern).unwrap_or_else(|err| {
        panic!("failed to compile regex {pattern:?}: {err}");
    });
    regex
        .captures(actual)
        .unwrap_or_else(|| panic!("regex {pattern:?} did not match {actual:?}"))
}

pub fn test_path_buf(unix_path: &str) -> PathBuf {
    PathBuf::from(unix_path)
}

pub fn test_absolute_path(unix_path: &str) -> AbsolutePathBuf {
    AbsolutePathBuf::from_absolute_path(test_path_buf(unix_path))
        .expect("test path should be absolute")
}

pub trait TempDirExt {
    fn abs(&self) -> AbsolutePathBuf;
}

impl TempDirExt for TempDir {
    fn abs(&self) -> AbsolutePathBuf {
        self.path().abs()
    }
}

pub fn test_tmp_path() -> AbsolutePathBuf {
    test_absolute_path("/tmp")
}

pub fn test_tmp_path_buf() -> PathBuf {
    test_tmp_path().into_path_buf()
}

pub fn fetch_dotslash_file(
    dotslash_file: &std::path::Path,
    dotslash_cache: Option<&std::path::Path>,
) -> anyhow::Result<PathBuf> {
    let mut command = std::process::Command::new("dotslash");
    command.arg("--").arg("fetch").arg(dotslash_file);
    if let Some(dotslash_cache) = dotslash_cache {
        command.env("DOTSLASH_CACHE", dotslash_cache);
    }
    let output = command.output().with_context(|| {
        format!(
            "failed to run dotslash to fetch resource {}",
            dotslash_file.display()
        )
    })?;
    ensure!(
        output.status.success(),
        "dotslash fetch failed for {}: {}",
        dotslash_file.display(),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let fetched_path = String::from_utf8(output.stdout)
        .context("dotslash fetch output was not utf8")?
        .trim()
        .to_string();
    ensure!(!fetched_path.is_empty(), "dotslash fetch output was empty");
    let fetched_path = PathBuf::from(fetched_path);
    ensure!(
        fetched_path.is_file(),
        "dotslash returned non-file path: {}",
        fetched_path.display()
    );
    Ok(fetched_path)
}

pub async fn load_default_config_for_test(codex_home: &TempDir) -> Config {
    ConfigBuilder::default()
        .loader_overrides(LoaderOverrides::for_tests())
        .codex_home(codex_home.path().to_path_buf())
        .harness_overrides(default_test_overrides())
        .build()
        .await
        .expect("defaults for test should always succeed")
}

fn default_test_overrides() -> ConfigOverrides {
    ConfigOverrides::default()
}

pub async fn wait_for_event<F>(
    codex: &CodexThread,
    predicate: F,
) -> codex_protocol::protocol::EventMsg
where
    F: FnMut(&codex_protocol::protocol::EventMsg) -> bool,
{
    use tokio::time::Duration;
    wait_for_event_with_timeout(codex, predicate, Duration::from_secs(1)).await
}

pub async fn submit_thread_settings(
    codex: &CodexThread,
    thread_settings: codex_protocol::protocol::ThreadSettingsOverrides,
) -> anyhow::Result<()> {
    use codex_protocol::protocol::EventMsg;
    use codex_protocol::protocol::Op;
    use tokio::time::Duration;
    use tokio::time::timeout;

    let submission_id = codex.submit(Op::ThreadSettings { thread_settings }).await?;
    loop {
        let ev = timeout(Duration::from_secs(10), codex.next_event())
            .await
            .expect("timeout waiting for thread settings update")
            .expect("stream ended unexpectedly");
        if ev.id == submission_id {
            match ev.msg {
                EventMsg::ThreadSettingsApplied(_) => return Ok(()),
                EventMsg::Error(err) => panic!("thread settings update failed: {}", err.message),
                other => panic!("unexpected thread settings update event: {other:?}"),
            }
        }
    }
}

pub async fn wait_for_event_match<T, F>(codex: &CodexThread, matcher: F) -> T
where
    F: Fn(&codex_protocol::protocol::EventMsg) -> Option<T>,
{
    let ev = wait_for_event(codex, |ev| matcher(ev).is_some()).await;
    matcher(&ev).expect("EventMsg should match matcher predicate")
}

pub async fn wait_for_event_with_timeout<F>(
    codex: &CodexThread,
    mut predicate: F,
    wait_time: tokio::time::Duration,
) -> codex_protocol::protocol::EventMsg
where
    F: FnMut(&codex_protocol::protocol::EventMsg) -> bool,
{
    use tokio::time::Duration;
    use tokio::time::timeout;
    loop {
        let ev = timeout(wait_time.max(Duration::from_secs(10)), codex.next_event())
            .await
            .expect("timeout waiting for event")
            .expect("stream ended unexpectedly");
        if predicate(&ev.msg) {
            return ev.msg;
        }
    }
}

pub fn format_with_current_shell(command: &str) -> Vec<String> {
    codex_core::shell::default_user_shell().derive_exec_args(command, true)
}

pub fn format_with_current_shell_display(command: &str) -> String {
    let args = format_with_current_shell(command);
    shlex::try_join(args.iter().map(String::as_str)).expect("serialize current shell command")
}

pub fn format_with_current_shell_non_login(command: &str) -> Vec<String> {
    codex_core::shell::default_user_shell().derive_exec_args(command, false)
}

pub fn format_with_current_shell_display_non_login(command: &str) -> String {
    let args = format_with_current_shell_non_login(command);
    shlex::try_join(args.iter().map(String::as_str))
        .expect("serialize current shell command without login")
}

pub fn stdio_server_bin() -> Result<String, CargoBinError> {
    codex_utils_cargo_bin::cargo_bin("test_stdio_server").map(|p| p.to_string_lossy().to_string())
}

pub mod fs_wait {
    use anyhow::Result;
    use anyhow::anyhow;
    use notify::RecursiveMode;
    use notify::Watcher;
    use std::path::Path;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::sync::mpsc::RecvTimeoutError;
    use std::time::Duration;
    use std::time::Instant;
    use tokio::task;
    use walkdir::WalkDir;

    pub async fn wait_for_path_exists(
        path: impl Into<PathBuf>,
        timeout: Duration,
    ) -> Result<PathBuf> {
        let path = path.into();
        task::spawn_blocking(move || wait_for_path_exists_blocking(path, timeout)).await?
    }

    pub async fn wait_for_matching_file(
        root: impl Into<PathBuf>,
        timeout: Duration,
        predicate: impl FnMut(&Path) -> bool + Send + 'static,
    ) -> Result<PathBuf> {
        let root = root.into();
        task::spawn_blocking(move || {
            let mut predicate = predicate;
            blocking_find_matching_file(root, timeout, &mut predicate)
        })
        .await?
    }

    fn wait_for_path_exists_blocking(path: PathBuf, timeout: Duration) -> Result<PathBuf> {
        if path.exists() {
            return Ok(path);
        }

        let watch_root = nearest_existing_ancestor(&path);
        let (tx, rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })?;
        watcher.watch(&watch_root, RecursiveMode::Recursive)?;

        let deadline = Instant::now() + timeout;
        loop {
            if path.exists() {
                return Ok(path);
            }
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let remaining = deadline.saturating_duration_since(now);
            match rx.recv_timeout(remaining) {
                Ok(Ok(_event)) => {
                    if path.exists() {
                        return Ok(path);
                    }
                }
                Ok(Err(err)) => return Err(err.into()),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }

        if path.exists() {
            Ok(path)
        } else {
            Err(anyhow!("timed out waiting for {path:?}"))
        }
    }

    fn blocking_find_matching_file(
        root: PathBuf,
        timeout: Duration,
        predicate: &mut impl FnMut(&Path) -> bool,
    ) -> Result<PathBuf> {
        let root = wait_for_path_exists_blocking(root, timeout)?;

        if let Some(found) = scan_for_match(&root, predicate) {
            return Ok(found);
        }

        let (tx, rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })?;
        watcher.watch(&root, RecursiveMode::Recursive)?;

        let deadline = Instant::now() + timeout;

        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(remaining) {
                Ok(Ok(_event)) => {
                    if let Some(found) = scan_for_match(&root, predicate) {
                        return Ok(found);
                    }
                }
                Ok(Err(err)) => return Err(err.into()),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }

        if let Some(found) = scan_for_match(&root, predicate) {
            Ok(found)
        } else {
            Err(anyhow!("timed out waiting for matching file in {root:?}"))
        }
    }

    fn scan_for_match(root: &Path, predicate: &mut impl FnMut(&Path) -> bool) -> Option<PathBuf> {
        for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
            let path = entry.path();
            if !entry.file_type().is_file() {
                continue;
            }
            if predicate(path) {
                return Some(path.to_path_buf());
            }
        }
        None
    }

    fn nearest_existing_ancestor(path: &Path) -> PathBuf {
        let mut current = path;
        loop {
            if current.exists() {
                return current.to_path_buf();
            }
            match current.parent() {
                Some(parent) => current = parent,
                None => return PathBuf::from("."),
            }
        }
    }
}
