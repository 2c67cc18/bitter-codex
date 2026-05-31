use anyhow::Context;
use anyhow::Result;
use app_test_support::AppServerProcess;
use app_test_support::create_final_assistant_message_sse_response;
use app_test_support::create_mock_responses_server_sequence_unchecked;
use app_test_support::to_response;
use codex_app_server_protocol::AdditionalContextEntry;
use codex_app_server_protocol::AdditionalContextKind;
use codex_app_server_protocol::JSONRPCResponse;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::TurnStartResponse;
use codex_app_server_protocol::UserInput as V2UserInput;
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::timeout;
use wiremock::MockServer;

#[cfg(any(target_os = "macos", windows))]
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(60);
#[cfg(not(any(target_os = "macos", windows)))]
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(10);

#[tokio::test]
async fn turn_start_injects_additional_context_into_model_request() -> Result<()> {
    let responses = vec![create_final_assistant_message_sse_response("Done")?];
    let server = create_mock_responses_server_sequence_unchecked(responses).await;

    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let mut app_server = AppServerProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, app_server.initialize()).await??;

    let thread_request_id = app_server
        .send_thread_start_request(ThreadStartParams::default())
        .await?;
    let thread_response: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        app_server.read_stream_until_response_message(RequestId::Integer(thread_request_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response(thread_response)?;

    let turn_request_id = app_server
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![V2UserInput::Text {
                text: "Use the supplied context.".to_string(),
                text_elements: Vec::new(),
            }],
            additional_context: Some(HashMap::from([
                (
                    "selection".to_string(),
                    AdditionalContextEntry {
                        value: "selected text".to_string(),
                        kind: AdditionalContextKind::Untrusted,
                    },
                ),
                (
                    "app_state".to_string(),
                    AdditionalContextEntry {
                        value: "active pane".to_string(),
                        kind: AdditionalContextKind::Application,
                    },
                ),
            ])),
            ..Default::default()
        })
        .await?;
    let turn_response: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        app_server.read_stream_until_response_message(RequestId::Integer(turn_request_id)),
    )
    .await??;
    let _: TurnStartResponse = to_response(turn_response)?;

    timeout(
        DEFAULT_READ_TIMEOUT,
        app_server.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    let body = responses_bodies(&server)
        .await?
        .into_iter()
        .next()
        .context("expected a responses request")?;

    let user_texts = message_input_texts(&body, "user");
    assert_eq!(
        user_texts
            .iter()
            .rev()
            .take(2)
            .rev()
            .cloned()
            .collect::<Vec<_>>(),
        vec![
            "<external_selection>selected text</external_selection>".to_string(),
            "Use the supplied context.".to_string(),
        ]
    );
    assert_eq!(
        message_input_texts(&body, "developer"),
        vec!["<app_state>active pane</app_state>".to_string()]
    );

    Ok(())
}

async fn responses_bodies(server: &MockServer) -> Result<Vec<Value>> {
    let requests = server
        .received_requests()
        .await
        .context("failed to fetch received requests")?;

    requests
        .into_iter()
        .filter(|req| req.url.path().ends_with("/responses"))
        .map(|req| {
            req.body_json::<Value>()
                .context("request body should be JSON")
        })
        .collect()
}

fn message_input_texts(body: &Value, role: &str) -> Vec<String> {
    body.get("input")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("role").and_then(Value::as_str) == Some(role))
        .filter_map(|item| item.get("content").and_then(Value::as_array))
        .flatten()
        .filter(|span| span.get("type").and_then(Value::as_str) == Some("input_text"))
        .filter_map(|span| span.get("text").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

fn create_config_toml(codex_home: &Path, server_uri: &str) -> std::io::Result<()> {
    let config_toml = codex_home.join("config.toml");
    std::fs::write(
        config_toml,
        format!(
            r#"
model = "mock-model"
approval_policy = "never"
sandbox_mode = "read-only"

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
