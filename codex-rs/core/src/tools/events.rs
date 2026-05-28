use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::turn_timing::now_unix_timestamp_ms;
use codex_protocol::exec_output::ExecToolCallOutput;
use codex_protocol::parse_command::ParsedCommand;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ExecCommandBeginEvent;
use codex_protocol::protocol::ExecCommandEndEvent;
use codex_protocol::protocol::ExecCommandSource;
use codex_protocol::protocol::ExecCommandStatus;
use codex_utils_absolute_path::AbsolutePathBuf;
use std::time::Duration;

use super::format_exec_output_str;

#[derive(Clone, Copy)]
pub(crate) struct ToolEventCtx<'a> {
    pub session: &'a Session,
    pub turn: &'a TurnContext,
    pub call_id: &'a str,
}

impl<'a> ToolEventCtx<'a> {
    pub fn new(
        session: &'a Session,
        turn: &'a TurnContext,
        call_id: &'a str,
    ) -> Self {
        Self {
            session,
            turn,
            call_id,
        }
    }
}

pub(crate) enum ToolEventStage {
    Begin,
    Success { output: ExecToolCallOutput },
    Failure(ToolEventFailure),
}

pub(crate) enum ToolEventFailure {
    Output(ExecToolCallOutput),
}

pub(crate) async fn emit_exec_command_begin(
    ctx: ToolEventCtx<'_>,
    command: &[String],
    cwd: &AbsolutePathBuf,
    parsed_cmd: &[ParsedCommand],
    source: ExecCommandSource,
    interaction_input: Option<String>,
    process_id: Option<&str>,
) {
    ctx.session
        .send_event(
            ctx.turn,
            EventMsg::ExecCommandBegin(ExecCommandBeginEvent {
                call_id: ctx.call_id.to_string(),
                process_id: process_id.map(str::to_owned),
                turn_id: ctx.turn.sub_id.clone(),
                started_at_ms: now_unix_timestamp_ms(),
                command: command.to_vec(),
                cwd: cwd.clone(),
                parsed_cmd: parsed_cmd.to_vec(),
                source,
                interaction_input,
            }),
        )
        .await;
}

pub(crate) enum ToolEmitter {
    UnifiedExec {
        command: Vec<String>,
        cwd: AbsolutePathBuf,
        source: ExecCommandSource,
        parsed_cmd: Vec<ParsedCommand>,
        process_id: Option<String>,
    },
}

impl ToolEmitter {

    pub fn unified_exec(
        command: &[String],
        cwd: AbsolutePathBuf,
        source: ExecCommandSource,
        process_id: Option<String>,
    ) -> Self {
        Self::UnifiedExec {
            command: command.to_vec(),
            cwd,
            source,
            parsed_cmd: vec![ParsedCommand::Unknown {
                cmd: command.join(" "),
            }],
            process_id,
        }
    }

    pub async fn emit(&self, ctx: ToolEventCtx<'_>, stage: ToolEventStage) {
        match (self, stage) {
            (
                Self::UnifiedExec {
                    command,
                    cwd,
                    source,
                    parsed_cmd,
                    process_id,
                },
                stage,
            ) => {
                emit_exec_stage(
                    ctx,
                    ExecCommandInput::new(
                        command,
                        cwd,
                        parsed_cmd,
                        *source,
                        None,
                        process_id.as_deref(),
                    ),
                    stage,
                )
                .await;
            }
        }
    }

    pub async fn begin(&self, ctx: ToolEventCtx<'_>) {
        self.emit(ctx, ToolEventStage::Begin).await;
    }


}

struct ExecCommandInput<'a> {
    command: &'a [String],
    cwd: &'a AbsolutePathBuf,
    parsed_cmd: &'a [ParsedCommand],
    source: ExecCommandSource,
    interaction_input: Option<&'a str>,
    process_id: Option<&'a str>,
}

impl<'a> ExecCommandInput<'a> {
    fn new(
        command: &'a [String],
        cwd: &'a AbsolutePathBuf,
        parsed_cmd: &'a [ParsedCommand],
        source: ExecCommandSource,
        interaction_input: Option<&'a str>,
        process_id: Option<&'a str>,
    ) -> Self {
        Self {
            command,
            cwd,
            parsed_cmd,
            source,
            interaction_input,
            process_id,
        }
    }
}

struct ExecCommandResult {
    stdout: String,
    stderr: String,
    aggregated_output: String,
    exit_code: i32,
    duration: Duration,
    formatted_output: String,
    status: ExecCommandStatus,
}

async fn emit_exec_stage(
    ctx: ToolEventCtx<'_>,
    exec_input: ExecCommandInput<'_>,
    stage: ToolEventStage,
) {
    match stage {
        ToolEventStage::Begin => {
            emit_exec_command_begin(
                ctx,
                exec_input.command,
                exec_input.cwd,
                exec_input.parsed_cmd,
                exec_input.source,
                exec_input.interaction_input.map(str::to_owned),
                exec_input.process_id,
            )
            .await;
        }
        ToolEventStage::Success { output }
        | ToolEventStage::Failure(ToolEventFailure::Output(output)) => {
            let exec_result = ExecCommandResult {
                stdout: output.stdout.text.clone(),
                stderr: output.stderr.text.clone(),
                aggregated_output: output.aggregated_output.text.clone(),
                exit_code: output.exit_code,
                duration: output.duration,
                formatted_output: format_exec_output_str(&output, ctx.turn.truncation_policy),
                status: if output.exit_code == 0 {
                    ExecCommandStatus::Completed
                } else {
                    ExecCommandStatus::Failed
                },
            };
            emit_exec_end(ctx, exec_input, exec_result).await;
        }
    }
}

async fn emit_exec_end(
    ctx: ToolEventCtx<'_>,
    exec_input: ExecCommandInput<'_>,
    exec_result: ExecCommandResult,
) {
    ctx.session
        .send_event(
            ctx.turn,
            EventMsg::ExecCommandEnd(ExecCommandEndEvent {
                call_id: ctx.call_id.to_string(),
                process_id: exec_input.process_id.map(str::to_owned),
                turn_id: ctx.turn.sub_id.clone(),
                completed_at_ms: crate::turn_timing::now_unix_timestamp_ms(),
                command: exec_input.command.to_vec(),
                cwd: exec_input.cwd.clone(),
                parsed_cmd: exec_input.parsed_cmd.to_vec(),
                source: exec_input.source,
                interaction_input: exec_input.interaction_input.map(str::to_owned),
                stdout: exec_result.stdout,
                stderr: exec_result.stderr,
                aggregated_output: exec_result.aggregated_output,
                exit_code: exec_result.exit_code,
                duration: exec_result.duration,
                formatted_output: exec_result.formatted_output,
                status: exec_result.status,
            }),
        )
        .await;
}
