use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::parse_arguments;
use crate::tools::handlers::parse_arguments_with_base_path;
use crate::tools::registry::CoreToolRuntime;
use crate::tools::registry::ToolExecutor;
use crate::unified_exec::ExecCommandRequest;
use crate::unified_exec::UnifiedExecContext;
use crate::unified_exec::UnifiedExecProcessManager;
use codex_otel::SessionTelemetry;
use codex_otel::TOOL_CALL_UNIFIED_EXEC_METRIC;
use codex_tools::ToolName;
use codex_tools::ToolSpec;

use super::super::shell_spec::CommandToolOptions;
use super::super::shell_spec::create_exec_command_tool;
use super::ExecCommandArgs;
use super::get_command;

#[derive(Clone, Copy)]
pub(crate) struct ExecCommandHandlerOptions {
    pub(crate) allow_login_shell: bool,
}

pub struct ExecCommandHandler {
    options: ExecCommandHandlerOptions,
}

impl Default for ExecCommandHandler {
    fn default() -> Self {
        Self {
            options: ExecCommandHandlerOptions {
                allow_login_shell: false,
            },
        }
    }
}

impl ExecCommandHandler {
    pub(crate) fn new(options: ExecCommandHandlerOptions) -> Self {
        Self { options }
    }
}

#[async_trait::async_trait]
impl ToolExecutor<ToolInvocation> for ExecCommandHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain("exec_command")
    }

    fn spec(&self) -> ToolSpec {
        create_exec_command_tool(CommandToolOptions {
            allow_login_shell: self.options.allow_login_shell,
        })
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        true
    }

    async fn handle(
        &self,
        invocation: ToolInvocation,
    ) -> Result<Box<dyn crate::tools::context::ToolOutput>, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            call_id,
            payload,
            ..
        } = invocation;

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "exec_command handler received unsupported payload".to_string(),
                ));
            }
        };

        let manager: &UnifiedExecProcessManager = &session.services.unified_exec_manager;
        let context = UnifiedExecContext::new(session.clone(), turn.clone(), call_id.clone());
        let Some(turn_environment) = turn.environments.primary() else {
            return Err(FunctionCallError::RespondToModel(
                "unified exec is unavailable in this session".to_string(),
            ));
        };
        let cwd_args: ExecCommandArgs = parse_arguments(&arguments)?;
        let cwd = cwd_args
            .workdir
            .as_deref()
            .filter(|workdir| !workdir.is_empty())
            .map_or_else(
                || turn_environment.cwd.clone(),
                |workdir| turn_environment.cwd.join(workdir),
            );
        let args: ExecCommandArgs = parse_arguments_with_base_path(&arguments, &cwd)?;
        let process_id = manager.allocate_process_id().await;
        let resolved_command =
            get_command(&args, session.user_shell(), turn.config.allow_login_shell)
                .map_err(FunctionCallError::RespondToModel)?;
        let command = resolved_command.command;
        let shell_type = resolved_command.shell_type;
        let command_for_display = command.join(" ");

        let ExecCommandArgs {
            tty,
            yield_time_ms,
            max_output_tokens,
            ..
        } = args;

        emit_unified_exec_tty_metric(&turn.session_telemetry, tty);
        match manager
            .exec_command(
                ExecCommandRequest {
                    command,
                    shell_type,
                    process_id,
                    yield_time_ms,
                    max_output_tokens,
                    cwd,
                    tty,
                },
                &context,
            )
            .await
        {
            Ok(response) => Ok(boxed_tool_output(response)),
            Err(err) => Err(FunctionCallError::RespondToModel(format!(
                "exec_command failed for `{command_for_display}`: {err:?}"
            ))),
        }
    }
}

impl CoreToolRuntime for ExecCommandHandler {
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }
}

fn emit_unified_exec_tty_metric(session_telemetry: &SessionTelemetry, tty: bool) {
    session_telemetry.counter(
        TOOL_CALL_UNIFIED_EXEC_METRIC,
        1,
        &[("tty", if tty { "true" } else { "false" })],
    );
}
