#![cfg(not(target_os = "windows"))]

use anyhow::Context;
use anyhow::Result;
use codex_protocol::models::PermissionProfile;
use codex_protocol::protocol::AskForApproval;
use core_test_support::apps_test_server::AppsTestServer;
use core_test_support::apps_test_server::CALENDAR_EXTRACT_TEXT_TOOL_NAME;
use core_test_support::apps_test_server::DOCUMENT_EXTRACT_TEXT_RESOURCE_URI;
use core_test_support::apps_test_server::SEARCH_CALENDAR_EXTRACT_TEXT_TOOL as DOCUMENT_EXTRACT_TOOL;
use core_test_support::apps_test_server::SEARCH_CALENDAR_NAMESPACE as DOCUMENT_EXTRACT_NAMESPACE;
use core_test_support::apps_test_server::apps_enabled_builder;
use core_test_support::apps_test_server::recorded_apps_tool_call_by_name;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_function_call_with_namespace;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_sse_sequence;
use core_test_support::responses::sse;
use core_test_support::responses::start_mock_server;
use pretty_assertions::assert_eq;
use serde_json::json;
use wiremock::Mock;
use wiremock::ResponseTemplate;
use wiremock::matchers::body_json;
use wiremock::matchers::header;
use wiremock::matchers::method;
use wiremock::matchers::path;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codex_apps_file_params_upload_local_paths_before_mcp_tool_call() -> Result<()> {
    let server = start_mock_server().await;
    let apps_server = AppsTestServer::mount(&server).await?;

    Mock::given(method("POST"))
        .and(path("/files"))
        .and(header("chatgpt-account-id", "account_id"))
        .and(body_json(json!({
            "file_name": "report.txt",
            "file_size": 11,
            "use_case": "codex",
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "file_id": "file_123",
            "upload_url": format!("{}/upload/file_123", server.uri()),
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/upload/file_123"))
        .and(header("content-length", "11"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/files/file_123/uploaded"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "success",
            "download_url": format!("{}/download/file_123", server.uri()),
            "file_name": "report.txt",
            "mime_type": "text/plain",
            "file_size_bytes": 11,
        })))
        .expect(1)
        .mount(&server)
        .await;

    let call_id = "extract-call-1";
    let mock = mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_response_created("resp-1"),
                ev_function_call_with_namespace(
                    call_id,
                    DOCUMENT_EXTRACT_NAMESPACE,
                    DOCUMENT_EXTRACT_TOOL,
                    &json!({"file": "report.txt"}).to_string(),
                ),
                ev_completed("resp-1"),
            ]),
            sse(vec![
                ev_response_created("resp-2"),
                ev_assistant_message("msg-1", "done"),
                ev_completed("resp-2"),
            ]),
        ],
    )
    .await;

    let builder = apps_enabled_builder(apps_server.chatgpt_base_url.clone());
    let test = builder.build(&server).await?;
    tokio::fs::write(test.cwd.path().join("report.txt"), b"hello world").await?;

    test.submit_turn_with_approval_and_permission_profile(
        "Extract the report text with the app tool.",
        AskForApproval::Never,
        PermissionProfile::Disabled,
    )
    .await?;

    let requests = mock.requests();
    let Some(extract_tool) =
        requests[0].tool_by_name(DOCUMENT_EXTRACT_NAMESPACE, DOCUMENT_EXTRACT_TOOL)
    else {
        let body = requests[0].body_json();
        panic!(
            "missing tool {DOCUMENT_EXTRACT_NAMESPACE}{DOCUMENT_EXTRACT_TOOL} in /v1/responses request: {body:?}"
        )
    };
    assert_eq!(
        extract_tool.pointer("/parameters/properties/file"),
        Some(&json!({
            "type": "string",
            "description": "Document file payload. This parameter expects an absolute local file path. If you want to upload a file, provide the absolute path to that file here."
        }))
    );

    let apps_tool_call =
        recorded_apps_tool_call_by_name(&server, CALENDAR_EXTRACT_TEXT_TOOL_NAME).await;

    assert_eq!(
        apps_tool_call.pointer("/params/arguments/file"),
        Some(&json!({
            "download_url": format!("{}/download/file_123", server.uri()),
            "file_id": "file_123",
            "mime_type": "text/plain",
            "file_name": "report.txt",
            "uri": "sediment://file_123",
            "file_size_bytes": 11,
        }))
    );
    assert_eq!(
        apps_tool_call.pointer("/params/_meta/_codex_apps"),
        Some(&json!({
            "call_id": "extract-call-1",
            "resource_uri": DOCUMENT_EXTRACT_TEXT_RESOURCE_URI,
            "contains_mcp_source": true,
            "connector_id": "calendar",
        }))
    );

    server.verify().await;
    Ok(())
}
