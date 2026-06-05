use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use serde_json::Value;
use serde_json::json;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandToolOptions {
    pub allow_login_shell: bool,
}

pub(crate) fn create_exec_command_tool(options: CommandToolOptions) -> ToolSpec {
    let mut properties = BTreeMap::from([
        (
            "cmd".to_string(),
            JsonSchema::string(Some("Shell command to execute.".to_string())),
        ),
        (
            "workdir".to_string(),
            JsonSchema::string(Some(
                "Working directory for the command. Defaults to the turn cwd.".to_string(),
            )),
        ),
        (
            "shell".to_string(),
            JsonSchema::string(Some(
                "Shell binary to launch. Defaults to the user's default shell.".to_string(),
            )),
        ),
        (
            "tty".to_string(),
            JsonSchema::boolean(Some(
                "True allocates a PTY for the command; false or omitted uses plain pipes."
                    .to_string(),
            )),
        ),
        (
            "yield_time_ms".to_string(),
            JsonSchema::number(Some(
                "Wait before yielding output. Defaults to 10000 ms; effective range is 250-30000 ms."
                    .to_string(),
            )),
        ),
        (
            "max_output_tokens".to_string(),
            JsonSchema::number(Some(
                "Output token budget. Defaults to 10000 tokens; larger requests may be capped by policy."
                    .to_string(),
            )),
        ),
    ]);
    if options.allow_login_shell {
        properties.insert(
            "login".to_string(),
            JsonSchema::boolean(Some(
                "True runs the shell with -l/-i semantics; false disables them. Defaults to true."
                    .to_string(),
            )),
        );
    }
    ToolSpec::Function(ResponsesApiTool {
        name: "exec_command".to_string(),
        description:
            "Runs a command in a PTY, returning output or a session ID for ongoing interaction."
                .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["cmd".to_string()]),
            Some(false.into()),
        ),
        output_schema: Some(unified_exec_output_schema()),
    })
}

pub fn create_write_stdin_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "session_id".to_string(),
            JsonSchema::number(Some(
                "Identifier of the running unified exec session.".to_string(),
            )),
        ),
        (
            "chars".to_string(),
            JsonSchema::string(Some(
                "Bytes to write to stdin. Defaults to empty, which polls without writing.".to_string(),
            )),
        ),
        (
            "yield_time_ms".to_string(),
            JsonSchema::number(Some(
                "Wait before yielding output. Non-empty writes default to 250 ms and cap at 30000 ms; empty polls wait 5000-300000 ms by default.".to_string(),
            )),
        ),
        (
            "max_output_tokens".to_string(),
            JsonSchema::number(Some(
                "Output token budget. Defaults to 10000 tokens; larger requests may be capped by policy."
                    .to_string(),
            )),
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "write_stdin".to_string(),
        description:
            "Writes characters to an existing unified exec session and returns recent output."
                .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["session_id".to_string()]),
            Some(false.into()),
        ),
        output_schema: Some(unified_exec_output_schema()),
    })
}

fn unified_exec_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "chunk_id": { "type": "string" },
            "wall_time_seconds": { "type": "number" },
            "exit_code": { "type": "number" },
            "session_id": { "type": "number" },
            "original_token_count": { "type": "number" },
            "output": { "type": "string" }
        },
        "required": ["wall_time_seconds", "output"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn exec_command_descriptions_match_retained_upstream_wording() {
        let ToolSpec::Function(tool) = create_exec_command_tool(CommandToolOptions {
            allow_login_shell: true,
        }) else {
            panic!("expected function tool spec");
        };
        let properties = tool.parameters.properties.expect("properties");

        assert_eq!(
            properties
                .get("workdir")
                .and_then(|schema| schema.description.as_deref()),
            Some("Working directory for the command. Defaults to the turn cwd.")
        );
        assert_eq!(
            properties
                .get("tty")
                .and_then(|schema| schema.description.as_deref()),
            Some("True allocates a PTY for the command; false or omitted uses plain pipes.")
        );
        assert_eq!(
            properties
                .get("yield_time_ms")
                .and_then(|schema| schema.description.as_deref()),
            Some(
                "Wait before yielding output. Defaults to 10000 ms; effective range is 250-30000 ms."
            )
        );
        assert_eq!(
            properties
                .get("max_output_tokens")
                .and_then(|schema| schema.description.as_deref()),
            Some(
                "Output token budget. Defaults to 10000 tokens; larger requests may be capped by policy."
            )
        );
        assert_eq!(
            properties
                .get("login")
                .and_then(|schema| schema.description.as_deref()),
            Some(
                "True runs the shell with -l/-i semantics; false disables them. Defaults to true."
            )
        );
    }

    #[test]
    fn write_stdin_descriptions_match_retained_upstream_wording() {
        let ToolSpec::Function(tool) = create_write_stdin_tool() else {
            panic!("expected function tool spec");
        };
        let properties = tool.parameters.properties.expect("properties");

        assert_eq!(
            properties
                .get("chars")
                .and_then(|schema| schema.description.as_deref()),
            Some("Bytes to write to stdin. Defaults to empty, which polls without writing.")
        );
        assert_eq!(
            properties
                .get("yield_time_ms")
                .and_then(|schema| schema.description.as_deref()),
            Some(
                "Wait before yielding output. Non-empty writes default to 250 ms and cap at 30000 ms; empty polls wait 5000-300000 ms by default."
            )
        );
        assert_eq!(
            properties
                .get("max_output_tokens")
                .and_then(|schema| schema.description.as_deref()),
            Some(
                "Output token budget. Defaults to 10000 tokens; larger requests may be capped by policy."
            )
        );
    }
}
