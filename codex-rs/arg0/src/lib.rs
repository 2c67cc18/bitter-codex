use std::fs::File;
use std::future::Future;
use std::path::Path;
use std::path::PathBuf;

use codex_utils_home_dir::find_codex_home;
use tempfile::TempDir;

const LOCK_FILENAME: &str = ".lock";
const TOKIO_WORKER_STACK_SIZE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Arg0DispatchPaths {
    pub codex_self_exe: Option<PathBuf>,
    pub main_execve_wrapper_exe: Option<PathBuf>,
}

pub struct Arg0PathEntryGuard {
    _temp_dir: TempDir,
    _lock_file: File,
    paths: Arg0DispatchPaths,
}

impl Arg0PathEntryGuard {
    fn new(temp_dir: TempDir, lock_file: File, paths: Arg0DispatchPaths) -> Self {
        Self {
            _temp_dir: temp_dir,
            _lock_file: lock_file,
            paths,
        }
    }

    pub fn paths(&self) -> &Arg0DispatchPaths {
        &self.paths
    }
}

pub fn arg0_dispatch() -> Option<Arg0PathEntryGuard> {
    load_dotenv();

    match prepend_path_entry_for_codex_aliases() {
        Ok(path_entry) => Some(path_entry),
        Err(err) => {
            eprintln!("WARNING: proceeding, even though we could not update PATH: {err}");
            None
        }
    }
}

pub fn arg0_dispatch_or_else<F, Fut>(main_fn: F) -> anyhow::Result<()>
where
    F: FnOnce(Arg0DispatchPaths) -> Fut,
    Fut: Future<Output = anyhow::Result<()>>,
{
    let path_entry_guard = arg0_dispatch();

    let runtime = build_runtime()?;
    runtime.block_on(run_main_with_arg0_guard(
        path_entry_guard,
        std::env::current_exe().ok(),
        main_fn,
    ))
}

async fn run_main_with_arg0_guard<F, Fut>(
    path_entry_guard: Option<Arg0PathEntryGuard>,
    current_exe: Option<PathBuf>,
    main_fn: F,
) -> anyhow::Result<()>
where
    F: FnOnce(Arg0DispatchPaths) -> Fut,
    Fut: Future<Output = anyhow::Result<()>>,
{
    let paths = Arg0DispatchPaths {
        codex_self_exe: current_exe.clone(),
        main_execve_wrapper_exe: None,
    };

    let result = main_fn(paths).await;

    drop(path_entry_guard);
    result
}

fn build_runtime() -> anyhow::Result<tokio::runtime::Runtime> {
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder.enable_all();
    builder.thread_stack_size(TOKIO_WORKER_STACK_SIZE_BYTES);
    Ok(builder.build()?)
}

const ILLEGAL_ENV_VAR_PREFIX: &str = "CODEX_";

fn load_dotenv() {
    if let Ok(codex_home) = find_codex_home()
        && let Ok(iter) = dotenvy::from_path_iter(codex_home.join(".env"))
    {
        set_filtered(iter);
    }
}

fn set_filtered<I>(iter: I)
where
    I: IntoIterator<Item = Result<(String, String), dotenvy::Error>>,
{
    for (key, value) in iter.into_iter().flatten() {
        if !key.to_ascii_uppercase().starts_with(ILLEGAL_ENV_VAR_PREFIX) {
            unsafe { std::env::set_var(&key, &value) };
        }
    }
}

pub fn prepend_path_entry_for_codex_aliases() -> std::io::Result<Arg0PathEntryGuard> {
    let codex_home = find_codex_home()?;
    #[cfg(not(debug_assertions))]
    {
        let temp_root = std::env::temp_dir();
        if codex_home.starts_with(&temp_root) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "Refusing to create helper binaries under temporary dir {temp_root:?} (codex_home: {codex_home:?})"
                ),
            ));
        }
    }

    std::fs::create_dir_all(&codex_home)?;

    let temp_root = codex_home.join("tmp").join("arg0");
    std::fs::create_dir_all(&temp_root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(&temp_root, std::fs::Permissions::from_mode(0o700))?;
    }

    if let Err(err) = janitor_cleanup(&temp_root) {
        eprintln!("WARNING: failed to clean up stale arg0 temp dirs: {err}");
    }

    let temp_dir = tempfile::Builder::new()
        .prefix("codex-arg0")
        .tempdir_in(&temp_root)?;
    let path = temp_dir.path();

    let lock_path = path.join(LOCK_FILENAME);
    let lock_file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)?;
    lock_file.try_lock()?;

    const PATH_SEPARATOR: &str = ":";

    let updated_path_env_var = match std::env::var_os("PATH") {
        Some(existing_path) => {
            let mut path_env_var =
                std::ffi::OsString::with_capacity(path.as_os_str().len() + 1 + existing_path.len());
            path_env_var.push(path);
            path_env_var.push(PATH_SEPARATOR);
            path_env_var.push(existing_path);
            path_env_var
        }
        None => path.as_os_str().to_owned(),
    };

    unsafe {
        std::env::set_var("PATH", updated_path_env_var);
    }

    let paths = Arg0DispatchPaths {
        codex_self_exe: std::env::current_exe().ok(),
        main_execve_wrapper_exe: None,
    };

    Ok(Arg0PathEntryGuard::new(temp_dir, lock_file, paths))
}

fn janitor_cleanup(temp_root: &Path) -> std::io::Result<()> {
    let entries = match std::fs::read_dir(temp_root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let Some(_lock_file) = try_lock_dir(&path)? else {
            continue;
        };

        match std::fs::remove_dir_all(&path) {
            Ok(()) => {}

            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err),
        }
    }

    Ok(())
}

fn try_lock_dir(dir: &Path) -> std::io::Result<Option<File>> {
    let lock_path = dir.join(LOCK_FILENAME);
    let lock_file = match File::options().read(true).write(true).open(&lock_path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };

    match lock_file.try_lock() {
        Ok(()) => Ok(Some(lock_file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::Arg0DispatchPaths;
    use super::Arg0PathEntryGuard;
    use super::LOCK_FILENAME;
    use super::janitor_cleanup;
    #[cfg(unix)]
    use super::run_main_with_arg0_guard;
    use std::fs;
    use std::fs::File;
    use std::path::Path;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn create_lock(dir: &Path) -> std::io::Result<File> {
        let lock_path = dir.join(LOCK_FILENAME);
        File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
    }

    #[cfg(unix)]
    #[test]
    fn run_main_with_arg0_guard_passes_current_exe() -> anyhow::Result<()> {
        let temp_dir = TempDir::new()?;
        let lock_file = create_lock(temp_dir.path())?;
        let path_entry = Arg0PathEntryGuard::new(
            temp_dir,
            lock_file,
            Arg0DispatchPaths {
                codex_self_exe: Some(PathBuf::from("/usr/bin/codex")),
                main_execve_wrapper_exe: None,
            },
        );

        super::build_runtime()?.block_on(run_main_with_arg0_guard(
            Some(path_entry),
            Some(PathBuf::from("/usr/bin/codex")),
            |paths| async move {
                assert_eq!(paths.codex_self_exe, Some(PathBuf::from("/usr/bin/codex")));
                assert_eq!(paths.main_execve_wrapper_exe, None);
                Ok(())
            },
        ))
    }

    #[test]
    fn janitor_skips_dirs_without_lock_file() -> std::io::Result<()> {
        let root = tempfile::tempdir()?;
        let dir = root.path().join("no-lock");
        fs::create_dir(&dir)?;

        janitor_cleanup(root.path())?;

        assert!(dir.exists());
        Ok(())
    }

    #[test]
    fn janitor_skips_dirs_with_held_lock() -> std::io::Result<()> {
        let root = tempfile::tempdir()?;
        let dir = root.path().join("locked");
        fs::create_dir(&dir)?;
        let lock_file = create_lock(&dir)?;
        lock_file.try_lock()?;

        janitor_cleanup(root.path())?;

        assert!(dir.exists());
        Ok(())
    }

    #[test]
    fn janitor_removes_dirs_with_unlocked_lock() -> std::io::Result<()> {
        let root = tempfile::tempdir()?;
        let dir = root.path().join("stale");
        fs::create_dir(&dir)?;
        create_lock(&dir)?;

        janitor_cleanup(root.path())?;

        assert!(!dir.exists());
        Ok(())
    }
}
