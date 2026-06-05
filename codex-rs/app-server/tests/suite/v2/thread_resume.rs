use anyhow::Result;
use app_test_support::AppServerProcess;
use app_test_support::create_fake_rollout_with_text_elements;
use app_test_support::create_mock_responses_server_repeating_assistant;
use app_test_support::rollout_path;
use codex_app_server_protocol::JSONRPCError;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ThreadForkParams;
use codex_app_server_protocol::ThreadResumeParams;
use codex_core::ARCHIVED_SESSIONS_SUBDIR;
use tempfile::TempDir;
use tokio::time::timeout;

const DEFAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[tokio::test]
async fn thread_resume_rejects_archived_session_by_id() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;
    let thread_id = create_archived_thread(codex_home.path())?;

    let mut app_server = AppServerProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, app_server.initialize()).await??;

    let resume_id = app_server
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: thread_id.clone(),
            ..Default::default()
        })
        .await?;

    let resume_err: JSONRPCError = timeout(
        DEFAULT_READ_TIMEOUT,
        app_server.read_stream_until_error_message(RequestId::Integer(resume_id)),
    )
    .await??;
    assert_archived_thread_error(resume_err, &thread_id);

    Ok(())
}

#[tokio::test]
async fn thread_fork_rejects_archived_session_by_id() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;
    let thread_id = create_archived_thread(codex_home.path())?;

    let mut app_server = AppServerProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, app_server.initialize()).await??;

    let fork_id = app_server
        .send_thread_fork_request(ThreadForkParams {
            thread_id: thread_id.clone(),
            ..Default::default()
        })
        .await?;

    let fork_err: JSONRPCError = timeout(
        DEFAULT_READ_TIMEOUT,
        app_server.read_stream_until_error_message(RequestId::Integer(fork_id)),
    )
    .await??;
    assert_archived_thread_error(fork_err, &thread_id);

    Ok(())
}

fn create_archived_thread(codex_home: &std::path::Path) -> Result<String> {
    let filename_ts = "2025-01-05T12-00-00";
    let thread_id = create_fake_rollout_with_text_elements(
        codex_home,
        filename_ts,
        "2025-01-05T12:00:00Z",
        "Archived saved user message",
        Vec::new(),
        Some("mock_provider"),
        None,
    )?;
    let active_rollout_path = rollout_path(codex_home, filename_ts, &thread_id);
    let archived_dir = codex_home.join(ARCHIVED_SESSIONS_SUBDIR);
    std::fs::create_dir_all(&archived_dir)?;
    std::fs::rename(
        &active_rollout_path,
        archived_dir.join(
            active_rollout_path
                .file_name()
                .expect("rollout path should include a file name"),
        ),
    )?;
    Ok(thread_id)
}

fn assert_archived_thread_error(error: JSONRPCError, thread_id: &str) {
    let message = error.error.message;
    assert!(
        message.contains(&format!("session {thread_id} is archived"))
            && message.contains(&format!(
                "`codex unarchive {thread_id}` to unarchive it first"
            )),
        "unexpected archived thread error: {message}"
    );
}

fn create_config_toml(codex_home: &std::path::Path, server_uri: &str) -> std::io::Result<()> {
    let config_toml = codex_home.join("config.toml");
    std::fs::write(
        config_toml,
        format!(
            r#"
model = "mock-model"
approval_policy = "never"
sandbox_mode = "danger-full-access"

model_provider = "mock_provider"

[model_providers.mock_provider]
name = "Mock provider for test"
base_url = "{server_uri}/v1"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0
"#
        ),
    )
}
