use codex_protocol::error::CodexErr;

#[derive(Debug)]
pub(crate) enum ToolError {
    Codex(CodexErr),
    Rejected(String),
}
