use anyhow::Context;
use anyhow::Result;
use app_test_support::McpProcess;
use app_test_support::create_final_assistant_message_sse_response;
use app_test_support::create_mock_responses_server_sequence_unchecked;
use app_test_support::to_response;
use codex_app_server_protocol::DynamicToolSpec;
use codex_app_server_protocol::JSONRPCResponse;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::TurnStartResponse;
use codex_app_server_protocol::UserInput as V2UserInput;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;
use std::path::Path;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::timeout;
use wiremock::MockServer;

// macOS and Windows Bazel CI can spend tens of seconds starting app-server
// subprocesses or processing test RPCs under load.
#[cfg(any(target_os = "macos", windows))]
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(60);
#[cfg(not(any(target_os = "macos", windows)))]
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Ensures dynamic tool specs are serialized into the model request payload.
#[tokio::test]
async fn thread_start_injects_dynamic_tools_into_model_requests() -> Result<()> {
    let responses = vec![create_final_assistant_message_sse_response("Done")?];
    let server = create_mock_responses_server_sequence_unchecked(responses).await;

    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    // Use a minimal JSON schema so we can assert the tool payload round-trips.
    let input_schema = json!({
        "type": "object",
        "properties": {
            "city": { "type": "string" }
        },
        "required": ["city"],
        "additionalProperties": false,
    });
    let dynamic_tool = DynamicToolSpec {
        namespace: None,
        name: "demo_tool".to_string(),
        description: "Demo dynamic tool".to_string(),
        input_schema: input_schema.clone(),
        defer_loading: false,
    };

    // Thread start injects dynamic tools into the thread's tool registry.
    let thread_req = mcp
        .send_thread_start_request(ThreadStartParams {
            dynamic_tools: Some(vec![dynamic_tool.clone()]),
            ..Default::default()
        })
        .await?;
    let thread_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(thread_req)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(thread_resp)?;

    // Start a turn so a model request is issued.
    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![V2UserInput::Text {
                text: "Hello".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    let turn_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_req)),
    )
    .await??;
    let _turn: TurnStartResponse = to_response::<TurnStartResponse>(turn_resp)?;

    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    // Inspect the captured model request to assert the tool spec made it through.
    let bodies = responses_bodies(&server).await?;
    let body = bodies
        .first()
        .context("expected at least one responses request")?;
    let tool = find_tool(body, &dynamic_tool.name)
        .context("expected dynamic tool to be injected into request")?;

    assert_eq!(
        tool.get("description"),
        Some(&Value::String(dynamic_tool.description.clone()))
    );
    assert_eq!(tool.get("parameters"), Some(&input_schema));

    Ok(())
}

#[tokio::test]
async fn thread_start_keeps_hidden_dynamic_tools_out_of_model_requests() -> Result<()> {
    let responses = vec![create_final_assistant_message_sse_response("Done")?];
    let server = create_mock_responses_server_sequence_unchecked(responses).await;

    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let dynamic_tool = DynamicToolSpec {
        namespace: Some("codex_app".to_string()),
        name: "hidden_tool".to_string(),
        description: "Hidden dynamic tool".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "city": { "type": "string" }
            },
            "required": ["city"],
            "additionalProperties": false,
        }),
        defer_loading: true,
    };

    let thread_req = mcp
        .send_thread_start_request(ThreadStartParams {
            dynamic_tools: Some(vec![dynamic_tool.clone()]),
            ..Default::default()
        })
        .await?;
    let thread_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(thread_req)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(thread_resp)?;

    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id,
            input: vec![V2UserInput::Text {
                text: "Hello".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    let turn_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_req)),
    )
    .await??;
    let _turn: TurnStartResponse = to_response::<TurnStartResponse>(turn_resp)?;

    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    let bodies = responses_bodies(&server).await?;
    assert!(
        bodies
            .iter()
            .all(|body| find_tool(body, &dynamic_tool.name).is_none()),
        "hidden dynamic tool should not be sent to the model"
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

fn find_tool<'a>(body: &'a Value, name: &str) -> Option<&'a Value> {
    body.get("tools")
        .and_then(Value::as_array)
        .and_then(|tools| {
            tools
                .iter()
                .find(|tool| tool.get("name").and_then(Value::as_str) == Some(name))
        })
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
