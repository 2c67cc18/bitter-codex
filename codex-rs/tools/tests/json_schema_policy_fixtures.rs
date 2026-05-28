use codex_protocol::dynamic_tools::DynamicToolSpec;
use codex_tools::dynamic_tool_to_responses_api_tool;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;

struct FixtureFile {
    source: &'static str,
    tools: Vec<FixtureTool>,
}

struct FixtureTool {
    name: &'static str,
    description: &'static str,
    input_schema: Value,
    expected_preserved: Vec<ExpectedValue>,
    expected_pruned: Vec<String>,
    expected_dropped_fields: Vec<String>,
}

struct ExpectedValue {
    pointer: String,
    value: Value,
}

#[test]
fn json_schema_policy_fixtures_convert_to_responses_tools() {
    for fixture in generic_schema_fixtures() {
        for fixture_tool in &fixture.tools {
            let responses_tool = convert_fixture_tool(&fixture, fixture_tool);
            let parameters = serde_json::to_value(&responses_tool.parameters)
                .expect("responses parameters should serialize");

            let expected_fields = [
                (
                    "preserve the tool name",
                    json!(fixture_tool.name),
                    json!(responses_tool.name),
                ),
                (
                    "preserve the tool description",
                    json!(fixture_tool.description),
                    json!(responses_tool.description),
                ),
                (
                    "remain a strict:false tool",
                    json!(false),
                    json!(responses_tool.strict),
                ),
                (
                    "produce object-shaped parameters",
                    json!("object"),
                    parameters.get("type").cloned().unwrap_or(Value::Null),
                ),
            ];

            for (message, expected, actual) in expected_fields {
                assert_eq!(actual, expected, "{} should {message}", fixture_tool.name);
            }
            assert!(
                parameters.get("properties").is_some_and(Value::is_object),
                "{} should produce a parameters.properties object",
                fixture_tool.name
            );

            for expected in &fixture_tool.expected_preserved {
                assert_eq!(
                    parameters.pointer(&expected.pointer),
                    Some(&expected.value),
                    "{} should preserve {}",
                    fixture_tool.name,
                    expected.pointer
                );
            }

            for pointer in &fixture_tool.expected_pruned {
                assert!(
                    parameters.pointer(pointer).is_none(),
                    "{} should prune unreachable definition {pointer}",
                    fixture_tool.name
                );
            }

            for pointer in &fixture_tool.expected_dropped_fields {
                assert!(
                    fixture_tool.input_schema.pointer(pointer).is_some(),
                    "{} fixture should contain expected dropped field {pointer}",
                    fixture_tool.name
                );
                assert!(
                    parameters.pointer(pointer).is_none(),
                    "{} should drop field {pointer} after JsonSchema conversion",
                    fixture_tool.name
                );
            }
        }
    }
}

#[test]
fn json_schema_policy_oversized_schema_triggers_compaction() {
    let fixture = oversized_schema_fixture();
    let fixture_tool = fixture
        .tools
        .first()
        .expect("oversized fixture should contain a tool");
    let input_bytes = compact_json_len(&fixture_tool.input_schema);

    let responses_tool = convert_fixture_tool(&fixture, fixture_tool);
    let parameters =
        serde_json::to_value(&responses_tool.parameters).expect("responses parameters serialize");
    let output_bytes = compact_json_len(&parameters);

    assert!(
        output_bytes < input_bytes,
        "compaction should reduce schema size from {input_bytes} bytes"
    );

    let absent_pointers = [
        ("/description", "drop root description"),
        ("/properties/parent/description", "drop nested descriptions"),
        (
            "/$defs",
            "drop root definitions after stripping descriptions is insufficient",
        ),
    ];
    for (pointer, message) in absent_pointers {
        assert!(
            parameters.pointer(pointer).is_none(),
            "oversized schema should {message}"
        );
    }

    let expected_values = [
        (
            "/properties/parent",
            json!({}),
            "rewrite local refs before dropping root definitions",
        ),
        (
            "/properties/children/items",
            json!({}),
            "rewrite array local refs before dropping root definitions",
        ),
        (
            "/properties/markdown/type",
            json!("string"),
            "retain top-level argument shape",
        ),
        (
            "/properties/properties/type",
            json!("object"),
            "retain object argument shape",
        ),
    ];
    for (pointer, expected, message) in expected_values {
        assert_eq!(
            parameters.pointer(pointer),
            Some(&expected),
            "oversized schema should {message}"
        );
    }
}

fn convert_fixture_tool(
    fixture: &FixtureFile,
    fixture_tool: &FixtureTool,
) -> codex_tools::ResponsesApiTool {
    let name = &fixture_tool.name;
    let tool = DynamicToolSpec {
        namespace: Some(fixture.source.to_string()),
        name: name.to_string(),
        description: fixture_tool.description.to_string(),
        input_schema: fixture_tool.input_schema.clone(),
        defer_loading: false,
    };

    dynamic_tool_to_responses_api_tool(&tool)
        .unwrap_or_else(|err| panic!("convert {name} from {}: {err}", fixture.source))
}

fn compact_json_len(value: &Value) -> usize {
    serde_json::to_vec(value)
        .unwrap_or_else(|err| panic!("serialize compact JSON: {err}"))
        .len()
}

fn generic_schema_fixtures() -> Vec<FixtureFile> {
    vec![
        FixtureFile {
            source: "generic/document",
            tools: vec![FixtureTool {
                name: "document_create",
                description: "Create a document with metadata",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "title": { "type": "string" },
                        "metadata": { "$ref": "#/$defs/metadata" }
                    },
                    "$defs": {
                        "metadata": {
                            "type": "object",
                            "properties": {
                                "owner": { "type": "string" },
                                "priority": { "enum": ["low", "normal", "high"] }
                            }
                        },
                        "unused": {
                            "type": "object",
                            "properties": {
                                "debug": { "type": "boolean" }
                            }
                        }
                    }
                }),
                expected_preserved: vec![
                    ExpectedValue {
                        pointer: "/properties/title/type".to_string(),
                        value: json!("string"),
                    },
                    ExpectedValue {
                        pointer: "/$defs/metadata/properties/priority/enum/2".to_string(),
                        value: json!("high"),
                    },
                ],
                expected_pruned: vec!["/$defs/unused".to_string()],
                expected_dropped_fields: Vec::new(),
            }],
        },
        FixtureFile {
            source: "generic/messaging",
            tools: vec![FixtureTool {
                name: "message_schedule",
                description: "Schedule a message",
                input_schema: json!({
                    "type": "object",
                    "title": "Message schedule input",
                    "description": "Input for scheduling a generic message.",
                    "properties": {
                        "body": {
                            "type": "string",
                            "description": "Message body"
                        },
                        "send_at": {
                            "type": "string",
                            "format": "date-time",
                            "examples": ["2026-01-01T12:00:00Z"]
                        }
                    },
                    "required": ["body", "send_at"]
                }),
                expected_preserved: vec![
                    ExpectedValue {
                        pointer: "/properties/body/type".to_string(),
                        value: json!("string"),
                    },
                    ExpectedValue {
                        pointer: "/required/1".to_string(),
                        value: json!("send_at"),
                    },
                ],
                expected_pruned: Vec::new(),
                expected_dropped_fields: vec![
                    "/title".to_string(),
                    "/properties/send_at/examples".to_string(),
                ],
            }],
        },
    ]
}

fn oversized_schema_fixture() -> FixtureFile {
    let long_description = "generic schema description ".repeat(2000);
    let large_enum: Vec<Value> = (0..1000)
        .map(|index| json!(format!("choice-{index}")))
        .collect();
    let input_schema = json!({
        "type": "object",
        "description": long_description,
        "properties": {
            "parent": {
                "description": "Parent document reference.",
                "$ref": "#/$defs/parent"
            },
            "children": {
                "type": "array",
                "items": {
                    "$ref": "#/$defs/child"
                }
            },
            "markdown": {
                "type": "string",
                "description": "Markdown body."
            },
            "properties": {
                "type": "object",
                "additionalProperties": true
            }
        },
        "$defs": {
            "parent": {
                "type": "object",
                "description": "Parent definition.",
                "properties": {
                    "id": { "type": "string" },
                    "kind": { "enum": large_enum }
                }
            },
            "child": {
                "type": "object",
                "description": "Child definition.",
                "properties": {
                    "text": { "type": "string" }
                }
            }
        }
    });

    FixtureFile {
        source: "generic/oversized",
        tools: vec![FixtureTool {
            name: "document_create_page",
            description: "Create a document page",
            input_schema,
            expected_preserved: Vec::new(),
            expected_pruned: Vec::new(),
            expected_dropped_fields: Vec::new(),
        }],
    }
}
