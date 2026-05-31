use crate::session::Submission;
use async_channel::Receiver;
use codex_otel::set_parent_from_w3c_trace_context;
use tracing::Instrument;
use tracing::info_span;

use crate::session::SteerInputError;
use crate::session::TurnInput;
use crate::session::session::Session;
use crate::session::session::SessionSettingsUpdate;

use crate::tasks::CompactTask;
use codex_protocol::protocol::CodexErrorInfo;
use codex_protocol::protocol::ErrorEvent;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::ThreadSettingsAppliedEvent;
use codex_protocol::protocol::ThreadSettingsOverrides;
use codex_protocol::protocol::ThreadSettingsSnapshot;
use codex_protocol::protocol::TurnAbortReason;

use crate::context_manager::is_user_turn_boundary;
use codex_protocol::dynamic_tools::DynamicToolResponse;
use std::sync::Arc;
use tracing::debug;
use tracing::info;
use tracing::warn;

pub async fn interrupt(sess: &Arc<Session>) {
    sess.interrupt_task().await;
}

pub async fn clean_background_terminals(sess: &Arc<Session>) {
    sess.close_unified_exec_processes().await;
}

pub async fn user_input_or_turn(sess: &Arc<Session>, sub_id: String, op: Op) {
    user_input_or_turn_inner(sess, sub_id, op).await;
}

pub async fn update_thread_settings(
    sess: &Arc<Session>,
    sub_id: String,
    thread_settings: ThreadSettingsOverrides,
) {
    let updates = thread_settings_update(sess, thread_settings).await;
    let msg = match sess.update_settings(updates).await {
        Ok(()) => thread_settings_applied_event(sess).await,
        Err(err) => EventMsg::Error(ErrorEvent {
            message: format!("invalid thread settings override: {err}"),
            codex_error_info: Some(CodexErrorInfo::BadRequest),
        }),
    };
    sess.send_event_raw(Event { id: sub_id, msg }).await;
}

async fn thread_settings_update(
    _sess: &Session,
    thread_settings: ThreadSettingsOverrides,
) -> SessionSettingsUpdate {
    let ThreadSettingsOverrides {
        cwd,
        workspace_roots,
        model: _,
        effort: _,
        summary,
        service_tier,
    } = thread_settings;
    SessionSettingsUpdate {
        cwd,
        workspace_roots,
        reasoning_summary: summary,
        service_tier,
        ..Default::default()
    }
}

async fn thread_settings_applied_event(sess: &Session) -> EventMsg {
    let snapshot = {
        let state = sess.state.lock().await;
        state.session_configuration.thread_config_snapshot()
    };
    EventMsg::ThreadSettingsApplied(ThreadSettingsAppliedEvent {
        thread_settings: ThreadSettingsSnapshot {
            model: snapshot.model,
            model_provider_id: snapshot.model_provider_id,
            service_tier: snapshot.service_tier,
            cwd: snapshot.cwd,
            reasoning_effort: snapshot.reasoning_effort,
            reasoning_summary: snapshot.reasoning_summary,
        },
    })
}

pub(super) async fn user_input_or_turn_inner(sess: &Arc<Session>, sub_id: String, op: Op) {
    let Op::UserInput {
        items,
        environments,
        final_output_json_schema,
        responsesapi_client_metadata,
        additional_context,
        thread_settings,
        web_tool_runtime,
    } = op
    else {
        unreachable!();
    };
    let emit_thread_settings_applied = thread_settings != ThreadSettingsOverrides::default();
    let mut updates = if emit_thread_settings_applied {
        thread_settings_update(sess, thread_settings).await
    } else {
        SessionSettingsUpdate::default()
    };
    updates.final_output_json_schema = Some(final_output_json_schema);
    updates.environments = environments;
    updates.web_tool_runtime = web_tool_runtime;

    let Ok(current_context) = sess.new_turn_with_sub_id(sub_id.clone(), updates).await else {
        return;
    };
    if emit_thread_settings_applied {
        sess.send_event_raw(Event {
            id: sub_id.clone(),
            msg: thread_settings_applied_event(sess).await,
        })
        .await;
    }
    sess.maybe_emit_unknown_model_warning_for_turn(current_context.as_ref())
        .await;
    match sess
        .steer_input(
            items.clone(),
            additional_context.clone(),
            None,
            responsesapi_client_metadata.clone(),
        )
        .await
    {
        Ok(_) => {
            current_context.session_telemetry.user_prompt(&items);
        }
        Err(SteerInputError::NoActiveTurn(items)) => {
            if let Some(responsesapi_client_metadata) = responsesapi_client_metadata {
                current_context
                    .turn_metadata_state
                    .set_responsesapi_client_metadata(responsesapi_client_metadata);
            }
            current_context.session_telemetry.user_prompt(&items);
            let additional_context_input = {
                let mut state = sess.state.lock().await;
                state.additional_context.merge(additional_context)
            };
            let mut task_input = additional_context_input
                .into_iter()
                .map(TurnInput::ResponseInputItem)
                .collect::<Vec<_>>();
            if !items.is_empty() {
                task_input.push(TurnInput::UserInput(items));
            }
            sess.spawn_task(
                Arc::clone(&current_context),
                task_input,
                crate::tasks::RegularTask::new(),
            )
            .await;
        }
        Err(err) => {
            sess.send_event_raw(Event {
                id: sub_id,
                msg: EventMsg::Error(err.to_error_event()),
            })
            .await;
        }
    }
}

pub async fn reload_user_config(sess: &Arc<Session>) {
    sess.reload_user_config_layer().await;
}

pub async fn dynamic_tool_response(sess: &Arc<Session>, id: String, response: DynamicToolResponse) {
    sess.notify_dynamic_tool_response(&id, response).await;
}

pub async fn compact(sess: &Arc<Session>, sub_id: String) {
    let turn_context = sess.new_default_turn_with_sub_id(sub_id).await;

    sess.spawn_task(Arc::clone(&turn_context), Vec::new(), CompactTask)
        .await;
}

async fn shutdown_session_runtime(sess: &Arc<Session>) {
    sess.abort_all_tasks(TurnAbortReason::Interrupted).await;
    sess.services
        .unified_exec_manager
        .terminate_all_processes()
        .await;
}

async fn emit_thread_stop_lifecycle(sess: &Session) {
    let _ = sess;
}

pub async fn shutdown(sess: &Arc<Session>, sub_id: String) -> bool {
    shutdown_session_runtime(sess).await;
    info!("Shutting down Codex instance");
    let history = sess.clone_history().await;
    let turn_count = history
        .raw_items()
        .iter()
        .filter(|item| is_user_turn_boundary(item))
        .count();
    sess.services.session_telemetry.counter(
        "codex.conversation.turn.count",
        i64::try_from(turn_count).unwrap_or(0),
        &[],
    );

    emit_thread_stop_lifecycle(sess.as_ref()).await;

    if let Some(live_thread) = sess.live_thread()
        && let Err(e) = live_thread.shutdown().await
    {
        warn!("failed to shutdown thread persistence: {e}");
        let event = Event {
            id: sub_id.clone(),
            msg: EventMsg::Error(ErrorEvent {
                message: "Failed to shutdown thread persistence".to_string(),
                codex_error_info: Some(CodexErrorInfo::Other),
            }),
        };
        sess.send_event_raw(event).await;
    }

    let event = Event {
        id: sub_id,
        msg: EventMsg::ShutdownComplete,
    };
    sess.deliver_event_raw(event).await;
    true
}

pub(super) async fn submission_loop(
    sess: Arc<Session>,
    _config: Arc<crate::config::Config>,
    rx_sub: Receiver<Submission>,
) {
    let mut shutdown_received = false;
    while let Ok(sub) = rx_sub.recv().await {
        debug!(?sub, "Submission");
        let dispatch_span = submission_dispatch_span(&sub);
        let should_exit = async {
            match sub.op.clone() {
                Op::Interrupt => {
                    interrupt(&sess).await;
                    false
                }
                Op::CleanBackgroundTerminals => {
                    clean_background_terminals(&sess).await;
                    false
                }
                Op::UserInput { .. } => {
                    user_input_or_turn(&sess, sub.id.clone(), sub.op).await;
                    false
                }
                Op::ThreadSettings { thread_settings } => {
                    update_thread_settings(&sess, sub.id.clone(), thread_settings).await;
                    false
                }
                Op::DynamicToolResponse { id, response } => {
                    dynamic_tool_response(&sess, id, response).await;
                    false
                }
                Op::ReloadUserConfig => {
                    reload_user_config(&sess).await;
                    false
                }
                Op::Compact => {
                    compact(&sess, sub.id.clone()).await;
                    false
                }
                Op::Shutdown => shutdown(&sess, sub.id.clone()).await,
                _ => false,
            }
        }
        .instrument(dispatch_span)
        .await;
        if should_exit {
            shutdown_received = true;
            break;
        }
    }

    if !shutdown_received {
        shutdown_session_runtime(&sess).await;
        emit_thread_stop_lifecycle(sess.as_ref()).await;
    }
    debug!("Agent loop exited");
}

pub(super) fn submission_dispatch_span(sub: &Submission) -> tracing::Span {
    let op_name = sub.op.kind();
    let span_name = format!("op.dispatch.{op_name}");
    let dispatch_span = info_span!(
        "submission_dispatch",
        otel.name = span_name.as_str(),
        submission.id = sub.id.as_str(),
        codex.op = op_name
    );
    if let Some(trace) = sub.trace.as_ref()
        && !set_parent_from_w3c_trace_context(&dispatch_span, trace)
    {
        warn!(
            submission.id = sub.id.as_str(),
            "ignoring invalid submission trace carrier"
        );
    }
    dispatch_span
}
