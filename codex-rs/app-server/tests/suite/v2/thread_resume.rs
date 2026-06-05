use anyhow::Result;
use app_test_support::AppServerProcess;
use app_test_support::create_fake_rollout;
use app_test_support::create_fake_rollout_with_text_elements;
use app_test_support::create_mock_responses_server_repeating_assistant;
use app_test_support::rollout_path;
use app_test_support::to_response;
use codex_app_server_protocol::ItemStartedNotification;
use codex_app_server_protocol::JSONRPCError;
use codex_app_server_protocol::JSONRPCResponse;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ThreadForkParams;
use codex_app_server_protocol::ThreadItem;
use codex_app_server_protocol::ThreadReadParams;
use codex_app_server_protocol::ThreadReadResponse;
use codex_app_server_protocol::ThreadResumeInitialTurnsPageParams;
use codex_app_server_protocol::ThreadResumeParams;
use codex_app_server_protocol::ThreadResumeResponse;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::TurnItemsView;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::UserInput as V2UserInput;
use codex_core::ARCHIVED_SESSIONS_SUBDIR;
use pretty_assertions::assert_eq;
use std::path::Path;
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
async fn thread_resume_returns_initial_turns_page() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;
    let conversation_id = create_fake_rollout(
        codex_home.path(),
        "2025-01-05T12-00-00",
        "2025-01-05T12:00:00Z",
        "Saved user message",
        Some("mock_provider"),
        None,
    )?;

    let mut app_server = AppServerProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, app_server.initialize()).await??;

    let resume_id = app_server
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: conversation_id.clone(),
            initial_turns_page: Some(ThreadResumeInitialTurnsPageParams {
                limit: Some(1),
                sort_direction: None,
                items_view: Some(TurnItemsView::Summary),
            }),
            ..Default::default()
        })
        .await?;
    let resume_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        app_server.read_stream_until_response_message(RequestId::Integer(resume_id)),
    )
    .await??;
    let ThreadResumeResponse {
        thread,
        initial_turns_page,
        ..
    } = to_response::<ThreadResumeResponse>(resume_resp)?;

    assert_eq!(thread.id, conversation_id);
    let page = initial_turns_page.expect("resume should include requested initial turns page");
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.data[0].items_view, TurnItemsView::Summary);
    assert_eq!(page.next_cursor, None);
    assert!(page.backwards_cursor.is_some());

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

#[tokio::test]
async fn thread_resume_and_fork_reject_explicit_directory_path() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;
    let thread_id = create_fake_rollout(
        codex_home.path(),
        "2025-01-05T12-00-00",
        "2025-01-05T12:00:00Z",
        "Saved user message",
        Some("mock_provider"),
        None,
    )?;

    let mut app_server = AppServerProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, app_server.initialize()).await??;

    let resume_id = app_server
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: thread_id.clone(),
            path: Some(codex_home.path().to_path_buf()),
            ..Default::default()
        })
        .await?;
    let resume_err: JSONRPCError = timeout(
        DEFAULT_READ_TIMEOUT,
        app_server.read_stream_until_error_message(RequestId::Integer(resume_id)),
    )
    .await??;
    assert_directory_path_error(resume_err);

    let fork_id = app_server
        .send_thread_fork_request(ThreadForkParams {
            thread_id,
            path: Some(codex_home.path().to_path_buf()),
            ..Default::default()
        })
        .await?;
    let fork_err: JSONRPCError = timeout(
        DEFAULT_READ_TIMEOUT,
        app_server.read_stream_until_error_message(RequestId::Integer(fork_id)),
    )
    .await??;
    assert_directory_path_error(fork_err);

    Ok(())
}

#[tokio::test]
async fn turn_start_client_id_reaches_notifications_and_thread_history() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let mut app_server = AppServerProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, app_server.initialize()).await??;
    let thread_id = start_thread(&mut app_server).await?;

    let turn_id = app_server
        .send_turn_start_request(TurnStartParams {
            thread_id: thread_id.clone(),
            input: vec![V2UserInput::Text {
                text: "hello with id".to_string(),
                text_elements: Vec::new(),
            }],
            client_id: Some("client-message-1".to_string()),
            ..Default::default()
        })
        .await?;
    let _: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        app_server.read_stream_until_response_message(RequestId::Integer(turn_id)),
    )
    .await??;

    let started_notif = timeout(
        DEFAULT_READ_TIMEOUT,
        app_server.read_stream_until_notification_message("item/started"),
    )
    .await??;
    let started: ItemStartedNotification = serde_json::from_value(
        started_notif
            .params
            .expect("item/started params must be present"),
    )?;
    assert_user_message_client_id(&started.item, Some("client-message-1"));

    timeout(
        DEFAULT_READ_TIMEOUT,
        app_server.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    let read_id = app_server
        .send_thread_read_request(ThreadReadParams {
            thread_id,
            include_turns: true,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        app_server.read_stream_until_response_message(RequestId::Integer(read_id)),
    )
    .await??;
    let ThreadReadResponse { thread } = to_response::<ThreadReadResponse>(read_resp)?;
    let item = thread
        .turns
        .iter()
        .flat_map(|turn| &turn.items)
        .find(|item| {
            matches!(
                item,
                ThreadItem::UserMessage {
                    content,
                    ..
                } if matches!(
                    content.as_slice(),
                    [V2UserInput::Text { text, .. }] if text == "hello with id"
                )
            )
        })
        .expect("thread history should include user message");
    assert_user_message_client_id(item, Some("client-message-1"));

    Ok(())
}

fn create_archived_thread(codex_home: &Path) -> Result<String> {
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

fn assert_directory_path_error(error: JSONRPCError) {
    let message = error.error.message;
    assert!(
        message.contains("path is a directory"),
        "unexpected directory path error: {message}"
    );
    assert!(
        !message.contains("Is a directory"),
        "directory should be rejected before rollout reading: {message}"
    );
}

fn assert_user_message_client_id(item: &ThreadItem, expected: Option<&str>) {
    let ThreadItem::UserMessage { client_id, .. } = item else {
        panic!("expected user message, got {item:?}");
    };
    assert_eq!(client_id.as_deref(), expected);
}

async fn start_thread(app_server: &mut AppServerProcess) -> Result<String> {
    let req_id = app_server
        .send_thread_start_request(ThreadStartParams {
            model: Some("gpt-5.2".to_string()),
            ..Default::default()
        })
        .await?;
    let resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        app_server.read_stream_until_response_message(RequestId::Integer(req_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(resp)?;
    Ok(thread.id)
}

fn create_config_toml(codex_home: &Path, server_uri: &str) -> std::io::Result<()> {
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
