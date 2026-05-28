pub(crate) mod context;
pub(crate) mod events;
pub(crate) mod handlers;
pub(crate) mod hosted_spec;
pub(crate) mod parallel;
pub(crate) mod registry;
pub(crate) mod router;
pub(crate) mod runtimes;
pub(crate) mod spec_plan;

use std::borrow::Cow;

use codex_protocol::exec_output::ExecToolCallOutput;
use codex_tools::ToolName;
use codex_utils_output_truncation::TruncationPolicy;
use codex_utils_output_truncation::formatted_truncate_text;
pub use router::ToolRouter;

pub(crate) const TELEMETRY_PREVIEW_MAX_BYTES: usize = 2 * 1024;
pub(crate) const TELEMETRY_PREVIEW_MAX_LINES: usize = 64;
pub(crate) const TELEMETRY_PREVIEW_TRUNCATION_NOTICE: &str =
    "[... telemetry preview truncated ...]";

pub(crate) fn flat_tool_name(tool_name: &ToolName) -> Cow<'_, str> {
    match tool_name.namespace.as_deref() {
        Some(namespace) => {
            let mut name = String::with_capacity(namespace.len() + tool_name.name.len());
            name.push_str(namespace);
            name.push_str(&tool_name.name);
            Cow::Owned(name)
        }
        None => Cow::Borrowed(tool_name.name.as_str()),
    }
}


pub fn format_exec_output_str(
    exec_output: &ExecToolCallOutput,
    truncation_policy: TruncationPolicy,
) -> String {
    let content = build_content_with_timeout(exec_output);

    formatted_truncate_text(&content, truncation_policy)
}

fn build_content_with_timeout(exec_output: &ExecToolCallOutput) -> String {
    if exec_output.timed_out {
        format!(
            "command timed out after {} milliseconds\n{}",
            exec_output.duration.as_millis(),
            exec_output.aggregated_output.text
        )
    } else {
        exec_output.aggregated_output.text.clone()
    }
}
