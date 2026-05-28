use crate::protocol::v2::CommandAction;
use crate::protocol::v2::CommandExecutionStatus;
use crate::protocol::v2::ThreadItem;
use codex_protocol::protocol::ExecCommandBeginEvent;
use codex_protocol::protocol::ExecCommandEndEvent;

fn command_to_display(command: &[String]) -> String {
    command.join(" ")
}

pub fn build_command_execution_begin_item(payload: &ExecCommandBeginEvent) -> ThreadItem {
    ThreadItem::CommandExecution {
        id: payload.call_id.clone(),
        command: command_to_display(&payload.command),
        cwd: payload.cwd.clone(),
        process_id: payload.process_id.clone(),
        source: payload.source.into(),
        status: CommandExecutionStatus::InProgress,
        command_actions: payload
            .parsed_cmd
            .iter()
            .cloned()
            .map(|parsed| CommandAction::from_core_with_cwd(parsed, &payload.cwd))
            .collect(),
        aggregated_output: None,
        exit_code: None,
        duration_ms: None,
    }
}

pub fn build_command_execution_end_item(payload: &ExecCommandEndEvent) -> ThreadItem {
    let aggregated_output = if payload.aggregated_output.is_empty() {
        None
    } else {
        Some(payload.aggregated_output.clone())
    };
    let duration_ms = i64::try_from(payload.duration.as_millis()).unwrap_or(i64::MAX);

    ThreadItem::CommandExecution {
        id: payload.call_id.clone(),
        command: command_to_display(&payload.command),
        cwd: payload.cwd.clone(),
        process_id: payload.process_id.clone(),
        source: payload.source.into(),
        status: (&payload.status).into(),
        command_actions: payload
            .parsed_cmd
            .iter()
            .cloned()
            .map(|parsed| CommandAction::from_core_with_cwd(parsed, &payload.cwd))
            .collect(),
        aggregated_output,
        exit_code: Some(payload.exit_code),
        duration_ms: Some(duration_ms),
    }
}
