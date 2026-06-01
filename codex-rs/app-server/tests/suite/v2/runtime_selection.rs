use anyhow::Context;
use anyhow::Result;
use app_test_support::AppServerProcess;
use app_test_support::ChatGptAuthFixture;
use app_test_support::create_final_assistant_message_sse_response;
use app_test_support::create_mock_responses_server_sequence_unchecked;
use app_test_support::to_response;
use app_test_support::write_chatgpt_auth;
use codex_app_server_protocol::JSONRPCResponse;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::TurnStartResponse;
use codex_app_server_protocol::UserInput as V2UserInput;
use codex_config::types::AuthCredentialsStoreMode;
use pretty_assertions::assert_eq;
use serde_json::Value;
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
async fn api_key_auth_selects_hosted_web_search_only() -> Result<()> {
    let body = run_turn_and_capture_responses_body(AuthFixture::ApiKey).await?;

    assert_eq!(
        web_surface_counts(&body)?,
        WebSurfaceCounts {
            hosted_web_search: 1,
            local_web_namespace: 0,
            local_web_run_namespace: 0,
        }
    );

    Ok(())
}

#[tokio::test]
async fn chatgpt_auth_selects_local_web_only() -> Result<()> {
    let body = run_turn_and_capture_responses_body(AuthFixture::ChatGpt).await?;

    assert_eq!(
        web_surface_counts(&body)?,
        WebSurfaceCounts {
            hosted_web_search: 0,
            local_web_namespace: 0,
            local_web_run_namespace: 1,
        }
    );

    Ok(())
}

enum AuthFixture {
    ApiKey,
    ChatGpt,
}

#[derive(Debug, PartialEq, Eq)]
struct WebSurfaceCounts {
    hosted_web_search: usize,
    local_web_namespace: usize,
    local_web_run_namespace: usize,
}

async fn run_turn_and_capture_responses_body(auth_fixture: AuthFixture) -> Result<Value> {
    let responses = vec![create_final_assistant_message_sse_response("Done")?];
    let server = create_mock_responses_server_sequence_unchecked(responses).await;

    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri(), &auth_fixture)?;
    if matches!(auth_fixture, AuthFixture::ChatGpt) {
        write_chatgpt_auth(
            codex_home.path(),
            ChatGptAuthFixture::new("access-token")
                .refresh_token("refresh-token")
                .account_id("acct_123")
                .email("user@example.com")
                .plan_type("pro"),
            AuthCredentialsStoreMode::File,
        )?;
    }

    let mut app_server = AppServerProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, app_server.initialize()).await??;

    let thread_req = app_server
        .send_thread_start_request(ThreadStartParams::default())
        .await?;
    let thread_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        app_server.read_stream_until_response_message(RequestId::Integer(thread_req)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(thread_resp)?;

    let turn_req = app_server
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id,
            input: vec![V2UserInput::Text {
                text: "Search the web for today's weather.".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    let turn_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        app_server.read_stream_until_response_message(RequestId::Integer(turn_req)),
    )
    .await??;
    let _turn: TurnStartResponse = to_response::<TurnStartResponse>(turn_resp)?;

    timeout(
        DEFAULT_READ_TIMEOUT,
        app_server.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    first_responses_body(&server).await
}

async fn first_responses_body(server: &MockServer) -> Result<Value> {
    let requests = server
        .received_requests()
        .await
        .context("failed to fetch received requests")?;

    requests
        .into_iter()
        .find(|req| req.url.path().ends_with("/responses"))
        .context("expected a responses request")?
        .body_json::<Value>()
        .context("request body should be JSON")
}

fn web_surface_counts(body: &Value) -> Result<WebSurfaceCounts> {
    let tools = body
        .get("tools")
        .and_then(Value::as_array)
        .context("request should include tools array")?;

    Ok(WebSurfaceCounts {
        hosted_web_search: tools
            .iter()
            .filter(|tool| {
                tool.get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|tool_type| tool_type.starts_with("web_search"))
            })
            .count(),
        local_web_namespace: tools
            .iter()
            .filter(|tool| {
                tool.get("type").and_then(Value::as_str) == Some("namespace")
                    && tool.get("name").and_then(Value::as_str) == Some("web")
            })
            .count(),
        local_web_run_namespace: tools
            .iter()
            .filter(|tool| {
                tool.get("type").and_then(Value::as_str) == Some("namespace")
                    && tool.get("name").and_then(Value::as_str) == Some("web_run")
            })
            .count(),
    })
}

fn create_config_toml(
    codex_home: &Path,
    server_uri: &str,
    auth_fixture: &AuthFixture,
) -> std::io::Result<()> {
    let requires_openai_auth = match auth_fixture {
        AuthFixture::ApiKey => "",
        AuthFixture::ChatGpt => "requires_openai_auth = true\n",
    };
    let config_toml = codex_home.join("config.toml");
    std::fs::write(
        config_toml,
        format!(
            r#"
model = "mock-model"
approval_policy = "never"
sandbox_mode = "read-only"
web_search = "live"

model_provider = "mock_provider"

[model_providers.mock_provider]
name = "Mock provider for test"
base_url = "{server_uri}/v1"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0
supports_websockets = false
{requires_openai_auth}
"#
        ),
    )
}
