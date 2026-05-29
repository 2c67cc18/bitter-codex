#![expect(clippy::expect_used)]

use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::process::Command;
use std::process::Stdio;
use tempfile::TempDir;

fn require_api_key() -> String {
    std::env::var("OPENAI_API_KEY")
        .expect("OPENAI_API_KEY env var not set — skip running live tests")
}

fn run_live(prompt: &str) -> (assert_cmd::assert::Assert, TempDir) {
    #![expect(clippy::unwrap_used)]
    use std::io::Read;
    use std::io::Write;
    use std::thread;

    let dir = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let codex_home = home.path().join(".bitter-codex");
    std::fs::create_dir_all(&codex_home).unwrap();

    let mut cmd = Command::new(codex_utils_cargo_bin::cargo_bin("codex-rs").unwrap());
    cmd.current_dir(dir.path());
    cmd.env("OPENAI_API_KEY", require_api_key());
    cmd.env("HOME", home.path());
    cmd.env("BITTER_CODEX_HOME", &codex_home);

    cmd.arg("--allow-no-git-exec")
        .arg("-v")
        .arg("--")
        .arg(prompt);

    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("failed to spawn codex-rs");

    child
        .stdin
        .as_mut()
        .expect("child stdin unavailable")
        .write_all(b"\n")
        .expect("failed to write to child stdin");

    fn tee<R: Read + Send + 'static>(
        mut reader: R,
        mut writer: impl Write + Send + 'static,
    ) -> thread::JoinHandle<Vec<u8>> {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        writer.write_all(&chunk[..n]).ok();
                        writer.flush().ok();
                        buf.extend_from_slice(&chunk[..n]);
                    }
                    Err(_) => break,
                }
            }
            buf
        })
    }

    let stdout_handle = tee(
        child.stdout.take().expect("child stdout"),
        std::io::stdout(),
    );
    let stderr_handle = tee(
        child.stderr.take().expect("child stderr"),
        std::io::stderr(),
    );

    let status = child.wait().expect("failed to wait on child");
    let stdout = stdout_handle.join().expect("stdout thread panicked");
    let stderr = stderr_handle.join().expect("stderr thread panicked");

    let output = std::process::Output {
        status,
        stdout,
        stderr,
    };

    (output.assert(), dir)
}

#[ignore]
#[test]
fn live_create_file_hello_txt() {
    if std::env::var("OPENAI_API_KEY").is_err() {
        eprintln!("skipping live_create_file_hello_txt – OPENAI_API_KEY not set");
        return;
    }

    let (assert, dir) = run_live(
        "Use the shell tool to create a file named hello.txt containing the text 'hello'.",
    );

    assert.success();

    let path = dir.path().join("hello.txt");
    assert!(path.exists(), "hello.txt was not created by the model");

    let contents = std::fs::read_to_string(path).unwrap();

    assert_eq!(contents.trim(), "hello");
}

#[ignore]
#[test]
fn live_print_working_directory() {
    if std::env::var("OPENAI_API_KEY").is_err() {
        eprintln!("skipping live_print_working_directory – OPENAI_API_KEY not set");
        return;
    }

    let (assert, dir) = run_live("Print the current working directory using the shell function.");

    assert
        .success()
        .stdout(predicate::str::contains(dir.path().to_string_lossy()));
}
