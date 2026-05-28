use assert_cmd::Command as AssertCommand;
use codex_git_utils::collect_git_info;
use codex_login::CODEX_API_KEY_ENV_VAR;
use codex_protocol::protocol::GitInfo;
use core_test_support::fs_wait;
use core_test_support::responses;
use std::time::Duration;
use tempfile::TempDir;
use uuid::Uuid;
use wiremock::MockServer;

fn repo_root() -> std::path::PathBuf {
    #[expect(clippy::expect_used)]
    codex_utils_cargo_bin::repo_root().expect("failed to resolve repo root")
}

fn cli_sse_response() -> String {
    responses::sse(vec![
        responses::ev_response_created("resp-fixture"),
        responses::ev_assistant_message("msg-fixture", "fixture hello"),
        responses::ev_completed("resp-fixture"),
    ])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn responses_mode_stream_cli() {
    let server = MockServer::start().await;
    let repo_root = repo_root();
    let sse = responses::sse(vec![
        responses::ev_response_created("resp-1"),
        responses::ev_assistant_message("msg-1", "hi"),
        responses::ev_completed("resp-1"),
    ]);
    let resp_mock = responses::mount_sse_once(&server, sse).await;

    let home = TempDir::new().unwrap();
    let provider_override = format!(
        "model_providers.mock={{ name = \"mock\", base_url = \"{}/v1\", env_key = \"PATH\", wire_api = \"responses\" }}",
        server.uri()
    );
    let bin = codex_utils_cargo_bin::cargo_bin("codex").unwrap();
    let mut cmd = AssertCommand::new(bin);
    cmd.timeout(Duration::from_secs(30));
    cmd.arg("exec")
        .arg("--skip-git-repo-check")
        .arg("-c")
        .arg(&provider_override)
        .arg("-c")
        .arg("model_provider=\"mock\"")
        .arg("-C")
        .arg(&repo_root)
        .arg("hello?");
    cmd.env("BITTER_CODEX_HOME", home.path())
        .env("OPENAI_API_KEY", "dummy");

    let output = cmd.output().unwrap();
    println!("Status: {}", output.status);
    println!("Stdout:\n{}", String::from_utf8_lossy(&output.stdout));
    println!("Stderr:\n{}", String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let hi_lines = stdout.lines().filter(|line| line.trim() == "hi").count();
    assert_eq!(hi_lines, 1, "Expected exactly one line with 'hi'");

    let request = resp_mock.single_request();
    assert_eq!(request.path(), "/v1/responses");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn responses_mode_stream_cli_supports_openai_base_url_config_override() {
    let server = MockServer::start().await;
    let repo_root = repo_root();
    let sse = responses::sse(vec![
        responses::ev_response_created("resp-1"),
        responses::ev_assistant_message("msg-1", "hi"),
        responses::ev_completed("resp-1"),
    ]);
    let resp_mock = responses::mount_sse_once(&server, sse).await;

    let home = TempDir::new().unwrap();
    let bin = codex_utils_cargo_bin::cargo_bin("codex").unwrap();
    let mut cmd = AssertCommand::new(bin);
    cmd.timeout(Duration::from_secs(30));
    cmd.arg("exec")
        .arg("--skip-git-repo-check")
        .arg("-c")
        .arg(format!("openai_base_url=\"{}/v1\"", server.uri()))
        .arg("-C")
        .arg(&repo_root)
        .arg("hello?");
    cmd.env("BITTER_CODEX_HOME", home.path())
        .env("OPENAI_API_KEY", "dummy");

    let output = cmd.output().unwrap();
    assert!(output.status.success());

    let request = resp_mock.single_request();
    assert_eq!(request.path(), "/v1/responses");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exec_cli_applies_model_instructions_file() {
    let server = MockServer::start().await;
    let sse = concat!(
        "data: {\"type\":\"response.created\",\"response\":{}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r1\"}}\n\n"
    );
    let resp_mock = core_test_support::responses::mount_sse_once(&server, sse.to_string()).await;

    let custom = TempDir::new().unwrap();
    let marker = "cli-model-instructions-file-marker";
    let custom_path = custom.path().join("instr.md");
    std::fs::write(&custom_path, marker).unwrap();
    let custom_path_str = custom_path.to_string_lossy().replace('\\', "/");

    let provider_override = format!(
        "model_providers.mock={{ name = \"mock\", base_url = \"{}/v1\", env_key = \"PATH\", wire_api = \"responses\" }}",
        server.uri()
    );

    let home = TempDir::new().unwrap();
    let repo_root = repo_root();
    let bin = codex_utils_cargo_bin::cargo_bin("codex").unwrap();
    let mut cmd = AssertCommand::new(bin);
    cmd.arg("exec")
        .arg("--skip-git-repo-check")
        .arg("-c")
        .arg(&provider_override)
        .arg("-c")
        .arg("model_provider=\"mock\"")
        .arg("-c")
        .arg(format!("model_instructions_file=\"{custom_path_str}\""))
        .arg("-C")
        .arg(&repo_root)
        .arg("hello?\n");
    cmd.env("BITTER_CODEX_HOME", home.path())
        .env("OPENAI_API_KEY", "dummy");

    let output = cmd.output().unwrap();
    println!("Status: {}", output.status);
    println!("Stdout:\n{}", String::from_utf8_lossy(&output.stdout));
    println!("Stderr:\n{}", String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success());

    let request = resp_mock.single_request();
    let body = request.body_json();
    let instructions = body
        .get("instructions")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    assert!(
        instructions.contains(marker),
        "instructions did not contain custom marker; got: {instructions}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn responses_api_stream_cli() {
    let server = MockServer::start().await;
    let resp_mock = responses::mount_sse_once(&server, cli_sse_response()).await;
    let repo_root = repo_root();

    let home = TempDir::new().unwrap();
    let bin = codex_utils_cargo_bin::cargo_bin("codex").unwrap();
    let mut cmd = AssertCommand::new(bin);
    cmd.timeout(Duration::from_secs(30));
    cmd.arg("exec")
        .arg("--skip-git-repo-check")
        .arg("-c")
        .arg(format!("openai_base_url=\"{}/v1\"", server.uri()))
        .arg("-C")
        .arg(&repo_root)
        .arg("hello?");
    cmd.env("BITTER_CODEX_HOME", home.path())
        .env("OPENAI_API_KEY", "dummy");

    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("fixture hello"));

    let request = resp_mock.single_request();
    assert_eq!(request.path(), "/v1/responses");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn integration_creates_and_checks_session_file() -> anyhow::Result<()> {
    let home = TempDir::new()?;

    let marker = format!("integration-test-{}", Uuid::new_v4());
    let prompt = format!("echo {marker}");

    let server = MockServer::start().await;
    let resp_mock =
        responses::mount_sse_sequence(&server, vec![cli_sse_response(), cli_sse_response()]).await;
    let repo_root = repo_root();

    let bin = codex_utils_cargo_bin::cargo_bin("codex").unwrap();
    let mut cmd = AssertCommand::new(bin);
    cmd.timeout(Duration::from_secs(30));
    cmd.arg("exec")
        .arg("--skip-git-repo-check")
        .arg("-c")
        .arg(format!("openai_base_url=\"{}/v1\"", server.uri()))
        .arg("-C")
        .arg(&repo_root)
        .arg(&prompt);
    cmd.env("BITTER_CODEX_HOME", home.path())
        .env(CODEX_API_KEY_ENV_VAR, "dummy");

    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "codex-cli exec failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let sessions_dir = home.path().join("sessions");
    fs_wait::wait_for_path_exists(&sessions_dir, Duration::from_secs(5)).await?;

    let marker_clone = marker.clone();
    let path = fs_wait::wait_for_matching_file(&sessions_dir, Duration::from_secs(10), move |p| {
        if p.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            return false;
        }
        let Ok(content) = std::fs::read_to_string(p) else {
            return false;
        };
        content.contains(&marker_clone)
    })
    .await?;

    let rel = match path.strip_prefix(&sessions_dir) {
        Ok(r) => r,
        Err(_) => panic!("session file should live under sessions/"),
    };
    let comps: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        comps.len(),
        4,
        "Expected sessions/YYYY/MM/DD/<file>, got {rel:?}"
    );
    let year = &comps[0];
    let month = &comps[1];
    let day = &comps[2];
    assert!(
        year.len() == 4 && year.chars().all(|c| c.is_ascii_digit()),
        "Year dir not 4-digit numeric: {year}"
    );
    assert!(
        month.len() == 2 && month.chars().all(|c| c.is_ascii_digit()),
        "Month dir not zero-padded 2-digit numeric: {month}"
    );
    assert!(
        day.len() == 2 && day.chars().all(|c| c.is_ascii_digit()),
        "Day dir not zero-padded 2-digit numeric: {day}"
    );
    if let Ok(m) = month.parse::<u8>() {
        assert!((1..=12).contains(&m), "Month out of range: {m}");
    }
    if let Ok(d) = day.parse::<u8>() {
        assert!((1..=31).contains(&d), "Day out of range: {d}");
    }

    let content =
        std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("Failed to read session file"));
    let mut lines = content.lines();
    let meta_line = lines
        .next()
        .ok_or("missing session meta line")
        .unwrap_or_else(|_| panic!("missing session meta line"));
    let meta: serde_json::Value = serde_json::from_str(meta_line)
        .unwrap_or_else(|_| panic!("Failed to parse session meta line as JSON"));
    assert_eq!(
        meta.get("type").and_then(|v| v.as_str()),
        Some("session_meta")
    );
    let payload = meta
        .get("payload")
        .unwrap_or_else(|| panic!("Missing payload in meta line"));
    assert!(payload.get("id").is_some(), "SessionMeta missing id");
    assert!(
        payload.get("timestamp").is_some(),
        "SessionMeta missing timestamp"
    );

    let mut found_message = false;
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(item) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if item.get("type").and_then(|t| t.as_str()) == Some("response_item")
            && let Some(payload) = item.get("payload")
            && payload.get("type").and_then(|t| t.as_str()) == Some("message")
            && let Some(c) = payload.get("content")
            && c.to_string().contains(&marker)
        {
            found_message = true;
            break;
        }
    }
    assert!(
        found_message,
        "No message found in session file containing the marker"
    );

    let marker2 = format!("integration-resume-{}", Uuid::new_v4());
    let prompt2 = format!("echo {marker2}");
    let bin2 = codex_utils_cargo_bin::cargo_bin("codex").unwrap();
    let mut cmd2 = AssertCommand::new(bin2);
    cmd2.timeout(Duration::from_secs(30));
    cmd2.arg("exec")
        .arg("--skip-git-repo-check")
        .arg("-c")
        .arg(format!("openai_base_url=\"{}/v1\"", server.uri()))
        .arg("-C")
        .arg(&repo_root)
        .arg(&prompt2)
        .arg("resume")
        .arg("--last");
    cmd2.env("BITTER_CODEX_HOME", home.path())
        .env("OPENAI_API_KEY", "dummy");

    let output2 = cmd2.output().unwrap();
    assert!(output2.status.success(), "resume codex-cli run failed");
    assert_eq!(resp_mock.requests().len(), 2);

    let marker2_clone = marker2.clone();
    let resumed_path =
        fs_wait::wait_for_matching_file(&sessions_dir, Duration::from_secs(10), move |p| {
            if p.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                return false;
            }
            std::fs::read_to_string(p)
                .map(|content| content.contains(&marker2_clone))
                .unwrap_or(false)
        })
        .await?;

    assert_eq!(
        resumed_path, path,
        "resume should create a new session file"
    );

    let resumed_content = std::fs::read_to_string(&resumed_path)?;
    assert!(
        resumed_content.contains(&marker),
        "resumed file missing original marker"
    );
    assert!(
        resumed_content.contains(&marker2),
        "resumed file missing resumed marker"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn integration_git_info_unit_test() {
    let temp_dir = TempDir::new().unwrap();
    let git_repo = temp_dir.path().to_path_buf();
    let envs = vec![
        ("GIT_CONFIG_GLOBAL", "/dev/null"),
        ("GIT_CONFIG_NOSYSTEM", "1"),
    ];

    let init_output = std::process::Command::new("git")
        .envs(envs.clone())
        .args(["init"])
        .current_dir(&git_repo)
        .output()
        .unwrap();
    assert!(init_output.status.success(), "git init failed");

    std::process::Command::new("git")
        .envs(envs.clone())
        .args(["config", "user.name", "Integration Test"])
        .current_dir(&git_repo)
        .output()
        .unwrap();

    std::process::Command::new("git")
        .envs(envs.clone())
        .args(["config", "user.email", "test@example.com"])
        .current_dir(&git_repo)
        .output()
        .unwrap();

    let test_file = git_repo.join("test.txt");
    std::fs::write(&test_file, "integration test content").unwrap();

    std::process::Command::new("git")
        .envs(envs.clone())
        .args(["add", "."])
        .current_dir(&git_repo)
        .output()
        .unwrap();

    let commit_output = std::process::Command::new("git")
        .envs(envs.clone())
        .args(["commit", "-m", "Integration test commit"])
        .current_dir(&git_repo)
        .output()
        .unwrap();
    assert!(commit_output.status.success(), "git commit failed");

    std::process::Command::new("git")
        .envs(envs.clone())
        .args(["checkout", "-b", "integration-test-branch"])
        .current_dir(&git_repo)
        .output()
        .unwrap();

    std::process::Command::new("git")
        .envs(envs.clone())
        .args([
            "remote",
            "add",
            "origin",
            "https://github.com/example/integration-test.git",
        ])
        .current_dir(&git_repo)
        .output()
        .unwrap();

    let git_info = collect_git_info(&git_repo).await;

    assert!(git_info.is_some(), "Git info should be collected");

    let git_info = git_info.unwrap();

    assert!(
        git_info.commit_hash.is_some(),
        "Git info should contain commit_hash"
    );
    let commit_hash = &git_info.commit_hash.as_ref().unwrap().0;
    assert_eq!(commit_hash.len(), 40, "Commit hash should be 40 characters");
    assert!(
        commit_hash.chars().all(|c| c.is_ascii_hexdigit()),
        "Commit hash should be hexadecimal"
    );

    assert!(git_info.branch.is_some(), "Git info should contain branch");
    let branch = git_info.branch.as_ref().unwrap();
    assert_eq!(
        branch, "integration-test-branch",
        "Branch should match what we created"
    );

    assert!(
        git_info.repository_url.is_some(),
        "Git info should contain repository_url"
    );
    let repo_url = git_info.repository_url.as_ref().unwrap();

    let expected_remote_url = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(&git_repo)
        .output()
        .unwrap();
    let expected_remote_url = String::from_utf8(expected_remote_url.stdout)
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(
        repo_url, &expected_remote_url,
        "Repository URL should match git remote get-url output"
    );

    println!("✅ Git info collection test passed!");
    println!("   Commit: {commit_hash}");
    println!("   Branch: {branch}");
    println!("   Repo: {repo_url}");

    let serialized = serde_json::to_string(&git_info).unwrap();
    let deserialized: GitInfo = serde_json::from_str(&serialized).unwrap();

    assert_eq!(git_info.commit_hash, deserialized.commit_hash);
    assert_eq!(git_info.branch, deserialized.branch);
    assert_eq!(git_info.repository_url, deserialized.repository_url);

    println!("✅ Git info serialization test passed!");
}
