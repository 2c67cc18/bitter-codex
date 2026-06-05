use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::function_tool::FunctionCallError;
use crate::session::turn_context::TurnContext;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::flat_tool_name;
use crate::util::error_or_panic;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::protocol::EventMsg;
use codex_tools::ToolName;
use futures::future::BoxFuture;

pub(crate) type ToolTelemetryTags = Vec<(&'static str, String)>;

pub use codex_tools::ToolExecutor;
pub use codex_tools::ToolExposure;

pub(crate) trait CoreToolRuntime: ToolExecutor<ToolInvocation> {
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }

    fn waits_for_runtime_cancellation(&self) -> bool {
        false
    }

    fn telemetry_tags<'a>(
        &'a self,
        _invocation: &'a ToolInvocation,
    ) -> BoxFuture<'a, ToolTelemetryTags> {
        Box::pin(async { Vec::new() })
    }
}

pub(crate) trait ToolArgumentDiffConsumer: Send {
    fn consume_diff(&mut self, turn: &TurnContext, call_id: String, diff: &str)
    -> Option<EventMsg>;

    fn finish(&mut self) -> Result<Option<EventMsg>, FunctionCallError> {
        Ok(None)
    }
}

pub(crate) struct AnyToolResult {
    pub(crate) call_id: String,
    pub(crate) payload: ToolPayload,
    pub(crate) result: Box<dyn ToolOutput>,
}

impl AnyToolResult {
    pub(crate) fn into_response(self) -> ResponseInputItem {
        let Self {
            call_id,
            payload,
            result,
            ..
        } = self;
        result.to_response_item(&call_id, &payload)
    }
}

pub struct ToolRegistry {
    tools: HashMap<ToolName, Arc<dyn CoreToolRuntime>>,
}

impl ToolRegistry {
    fn new(tools: HashMap<ToolName, Arc<dyn CoreToolRuntime>>) -> Self {
        Self { tools }
    }

    pub(crate) fn from_tools(tools: impl IntoIterator<Item = Arc<dyn CoreToolRuntime>>) -> Self {
        let mut tools_by_name = HashMap::new();
        for tool in tools {
            let name = tool.tool_name();
            if tools_by_name.contains_key(&name) {
                error_or_panic(format!("tool {name} already registered"));
                continue;
            }
            tools_by_name.insert(name, tool);
        }
        Self::new(tools_by_name)
    }

    fn tool(&self, name: &ToolName) -> Option<Arc<dyn CoreToolRuntime>> {
        self.tools.get(name).map(Arc::clone)
    }

    pub(crate) fn supports_parallel_tool_calls(&self, name: &ToolName) -> Option<bool> {
        let tool = self.tool(name)?;
        Some(tool.supports_parallel_tool_calls())
    }

    pub(crate) fn waits_for_runtime_cancellation(&self, name: &ToolName) -> Option<bool> {
        let tool = self.tool(name)?;
        Some(tool.waits_for_runtime_cancellation())
    }

    #[allow(dead_code)]
    pub(crate) async fn dispatch_any(
        &self,
        invocation: ToolInvocation,
    ) -> Result<AnyToolResult, FunctionCallError> {
        self.dispatch_any_with_terminal_outcome(invocation, None)
            .await
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "tool dispatch must keep active-turn accounting atomic"
    )]
    pub(crate) async fn dispatch_any_with_terminal_outcome(
        &self,
        invocation: ToolInvocation,
        terminal_outcome_reached: Option<Arc<AtomicBool>>,
    ) -> Result<AnyToolResult, FunctionCallError> {
        let tool_name = invocation.tool_name.clone();
        let tool_name_flat = flat_tool_name(&tool_name);
        let call_id_owned = invocation.call_id.clone();
        let otel = invocation.turn.session_telemetry.clone();
        let base_tool_result_tags: Vec<(&'static str, &str)> = Vec::new();

        {
            let mut active = invocation.session.active_turn.lock().await;
            if let Some(active_turn) = active.as_mut() {
                let mut turn_state = active_turn.turn_state.lock().await;
                turn_state.tool_calls = turn_state.tool_calls.saturating_add(1);
            }
        }

        let tool = match self.tool(&tool_name) {
            Some(tool) => tool,
            None => {
                let message = unsupported_tool_call_message(&invocation.payload, &tool_name);
                let log_payload = invocation.payload.log_payload();
                otel.tool_result_with_tags(
                    tool_name_flat.as_ref(),
                    &call_id_owned,
                    log_payload.as_ref(),
                    Duration::ZERO,
                    false,
                    &message,
                    &base_tool_result_tags,
                );
                let err = FunctionCallError::RespondToModel(message);
                return Err(err);
            }
        };

        let telemetry_tags = tool.telemetry_tags(&invocation).await;
        let mut tool_result_tags =
            Vec::with_capacity(base_tool_result_tags.len() + telemetry_tags.len());
        tool_result_tags.extend_from_slice(&base_tool_result_tags);
        for (key, value) in &telemetry_tags {
            tool_result_tags.push((*key, value.as_str()));
        }
        if !tool.matches_kind(&invocation.payload) {
            let message = format!("tool {tool_name} invoked with incompatible payload");
            let log_payload = invocation.payload.log_payload();
            otel.tool_result_with_tags(
                tool_name_flat.as_ref(),
                &call_id_owned,
                log_payload.as_ref(),
                Duration::ZERO,
                false,
                &message,
                &tool_result_tags,
            );
            let err = FunctionCallError::Fatal(message);
            return Err(err);
        }

        let response_cell = tokio::sync::Mutex::new(None);
        let invocation_for_tool = invocation.clone();
        let log_payload = invocation.payload.log_payload();

        let result = otel
            .log_tool_result_with_tags(
                tool_name_flat.as_ref(),
                &call_id_owned,
                log_payload.as_ref(),
                &tool_result_tags,
                || {
                    let tool = tool.clone();
                    let response_cell = &response_cell;
                    async move {
                        match handle_any_tool(tool.as_ref(), invocation_for_tool).await {
                            Ok(result) => {
                                let preview = result.result.log_preview();
                                let success = result.result.success_for_logging();
                                let mut guard = response_cell.lock().await;
                                *guard = Some(result);
                                Ok((preview, success))
                            }
                            Err(err) => Err(err),
                        }
                    }
                },
            )
            .await;
        let _success = match &result {
            Ok((_, success)) => *success,
            Err(_) => false,
        };
        if let Some(terminal_outcome_reached) = &terminal_outcome_reached {
            terminal_outcome_reached.store(true, Ordering::Release);
        }

        match result {
            Ok(_) => {
                let mut guard = response_cell.lock().await;
                let result = guard.take().ok_or_else(|| {
                    FunctionCallError::Fatal("tool produced no output".to_string())
                })?;
                Ok(result)
            }
            Err(err) => Err(err),
        }
    }
}

async fn handle_any_tool(
    tool: &dyn CoreToolRuntime,
    invocation: ToolInvocation,
) -> Result<AnyToolResult, FunctionCallError> {
    let call_id = invocation.call_id.clone();
    let payload = invocation.payload.clone();
    let output = tool.handle(invocation.clone()).await?;
    Ok(AnyToolResult {
        call_id,
        payload,
        result: output,
    })
}

fn unsupported_tool_call_message(payload: &ToolPayload, tool_name: &ToolName) -> String {
    match payload {
        ToolPayload::Custom { .. } => format!("unsupported custom tool call: {tool_name}"),
        _ => format!("unsupported call: {tool_name}"),
    }
}
