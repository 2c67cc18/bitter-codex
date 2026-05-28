use crate::outgoing_message::ThreadScopedOutgoingMessageSender;
use crate::request_processors::thread_settings_from_core_snapshot;
use crate::thread_state::ThreadState;
use crate::thread_state::TurnSummary;
use crate::thread_status::ThreadWatchManager;
use codex_app_server_protocol::AccountRateLimitsUpdatedNotification;
use codex_app_server_protocol::CodexErrorInfo as V2CodexErrorInfo;
use codex_app_server_protocol::DeprecationNoticeNotification;
use codex_app_server_protocol::ErrorNotification;
use codex_app_server_protocol::ModelReroutedNotification;
use codex_app_server_protocol::ModelVerificationNotification;
use codex_app_server_protocol::ServerNotification;
use codex_app_server_protocol::ThreadSettingsUpdatedNotification;
use codex_app_server_protocol::ThreadTokenUsage;
use codex_app_server_protocol::ThreadTokenUsageUpdatedNotification;
use codex_app_server_protocol::Turn;
use codex_app_server_protocol::TurnCompletedNotification;
use codex_app_server_protocol::TurnError;
use codex_app_server_protocol::TurnInterruptResponse;
use codex_app_server_protocol::TurnItemsView;
use codex_app_server_protocol::TurnStartedNotification;
use codex_app_server_protocol::TurnStatus;
use codex_app_server_protocol::WarningNotification;
use codex_app_server_protocol::item_event_to_server_notification;
use codex_protocol::ThreadId;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::TokenCountEvent;
use codex_protocol::protocol::TurnAbortedEvent;
use codex_protocol::protocol::TurnCompleteEvent;
use std::sync::Arc;
use tokio::sync::Mutex;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn apply_bespoke_event_handling(
    event: Event,
    conversation_id: ThreadId,
    outgoing: ThreadScopedOutgoingMessageSender,
    thread_state: Arc<tokio::sync::Mutex<ThreadState>>,
    thread_watch_manager: ThreadWatchManager,
) {
    let Event {
        id: event_turn_id,
        msg,
    } = event;
    match msg {
        EventMsg::TurnStarted(payload) => {
            outgoing.abort_pending_server_requests().await;
            thread_watch_manager
                .note_turn_started(&conversation_id.to_string())
                .await;
            let turn = {
                let state = thread_state.lock().await;
                let mut turn = state.active_turn_snapshot().unwrap_or_else(|| Turn {
                    id: payload.turn_id.clone(),
                    items: Vec::new(),
                    items_view: TurnItemsView::NotLoaded,
                    error: None,
                    status: TurnStatus::InProgress,
                    started_at: payload.started_at,
                    completed_at: None,
                    duration_ms: None,
                });
                turn.items.clear();
                turn.items_view = TurnItemsView::NotLoaded;
                turn
            };
            let notification = TurnStartedNotification {
                thread_id: conversation_id.to_string(),
                turn,
            };
            outgoing
                .send_server_notification(ServerNotification::TurnStarted(notification))
                .await;
        }
        EventMsg::TurnComplete(turn_complete_event) => {
            outgoing.abort_pending_server_requests().await;
            respond_to_pending_interrupts(&thread_state, &outgoing).await;
            let turn_failed = thread_state.lock().await.turn_summary.last_error.is_some();
            thread_watch_manager
                .note_turn_completed(&conversation_id.to_string(), turn_failed)
                .await;
            handle_turn_complete(
                conversation_id,
                event_turn_id,
                turn_complete_event,
                &outgoing,
                &thread_state,
            )
            .await;
        }
        EventMsg::Warning(warning_event) => {
            let notification = WarningNotification {
                thread_id: Some(conversation_id.to_string()),
                message: warning_event.message,
            };
            outgoing
                .send_server_notification(ServerNotification::Warning(notification))
                .await;
        }
        EventMsg::ModelReroute(event) => {
            let notification = ModelReroutedNotification {
                thread_id: conversation_id.to_string(),
                turn_id: event_turn_id.clone(),
                from_model: event.from_model,
                to_model: event.to_model,
                reason: event.reason.into(),
            };
            outgoing
                .send_server_notification(ServerNotification::ModelRerouted(notification))
                .await;
        }
        EventMsg::ModelVerification(event) => {
            let notification = ModelVerificationNotification {
                thread_id: conversation_id.to_string(),
                turn_id: event_turn_id.clone(),
                verifications: event.verifications.into_iter().map(Into::into).collect(),
            };
            outgoing
                .send_server_notification(ServerNotification::ModelVerification(notification))
                .await;
        }
        msg @ (EventMsg::AgentMessageContentDelta(_)
        | EventMsg::ReasoningContentDelta(_)
        | EventMsg::ReasoningRawContentDelta(_)
        | EventMsg::AgentReasoningSectionBreak(_)) => {
            let notification = item_event_to_server_notification(
                msg,
                &conversation_id.to_string(),
                &event_turn_id,
            );
            outgoing.send_server_notification(notification).await;
        }
        EventMsg::DeprecationNotice(event) => {
            let notification = DeprecationNoticeNotification {
                summary: event.summary,
                details: event.details,
            };
            outgoing
                .send_server_notification(ServerNotification::DeprecationNotice(notification))
                .await;
        }
        EventMsg::TokenCount(token_count_event) => {
            handle_token_count_event(conversation_id, event_turn_id, token_count_event, &outgoing)
                .await;
        }
        EventMsg::Error(ev) => {
            thread_watch_manager
                .note_system_error(&conversation_id.to_string())
                .await;

            if !ev.affects_turn_status() {
                return;
            }

            let turn_error = TurnError {
                message: ev.message,
                codex_error_info: ev.codex_error_info.map(V2CodexErrorInfo::from),
                additional_details: None,
            };
            handle_error(conversation_id, turn_error.clone(), &thread_state).await;
            outgoing
                .send_server_notification(ServerNotification::Error(ErrorNotification {
                    error: turn_error.clone(),
                    will_retry: false,
                    thread_id: conversation_id.to_string(),
                    turn_id: event_turn_id.clone(),
                }))
                .await;
        }
        EventMsg::StreamError(ev) => {
            let turn_error = TurnError {
                message: ev.message,
                codex_error_info: ev.codex_error_info.map(V2CodexErrorInfo::from),
                additional_details: ev.additional_details,
            };
            outgoing
                .send_server_notification(ServerNotification::Error(ErrorNotification {
                    error: turn_error,
                    will_retry: true,
                    thread_id: conversation_id.to_string(),
                    turn_id: event_turn_id.clone(),
                }))
                .await;
        }
        msg @ (EventMsg::ItemStarted(_)
        | EventMsg::ItemCompleted(_)
        | EventMsg::TerminalInteraction(_)) => {
            let notification = item_event_to_server_notification(
                msg,
                &conversation_id.to_string(),
                &event_turn_id,
            );
            outgoing.send_server_notification(notification).await;
        }
        EventMsg::RawResponseItem(_) => {}
        EventMsg::ExecCommandBegin(exec_command_begin_event) => {
            if matches!(
                exec_command_begin_event.source,
                codex_protocol::protocol::ExecCommandSource::UnifiedExecInteraction
            ) {
                return;
            }
            let item_id = exec_command_begin_event.call_id.clone();
            let first_start = {
                let mut state = thread_state.lock().await;
                state
                    .turn_summary
                    .command_execution_started
                    .insert(item_id.clone())
            };
            if first_start {
                let notification = item_event_to_server_notification(
                    EventMsg::ExecCommandBegin(exec_command_begin_event),
                    &conversation_id.to_string(),
                    &event_turn_id,
                );
                outgoing.send_server_notification(notification).await;
            }
        }
        EventMsg::ExecCommandOutputDelta(exec_command_output_delta_event) => {
            let notification = item_event_to_server_notification(
                EventMsg::ExecCommandOutputDelta(exec_command_output_delta_event),
                &conversation_id.to_string(),
                &event_turn_id,
            );
            outgoing.send_server_notification(notification).await;
        }
        EventMsg::ExecCommandEnd(exec_command_end_event) => {
            let call_id = exec_command_end_event.call_id.clone();
            {
                let mut state = thread_state.lock().await;
                state
                    .turn_summary
                    .command_execution_started
                    .remove(&call_id);
            }
            if matches!(
                exec_command_end_event.source,
                codex_protocol::protocol::ExecCommandSource::UnifiedExecInteraction
            ) {
                return;
            }
            let notification = item_event_to_server_notification(
                EventMsg::ExecCommandEnd(exec_command_end_event),
                &conversation_id.to_string(),
                &event_turn_id,
            );
            outgoing.send_server_notification(notification).await;
        }

        EventMsg::TurnAborted(turn_aborted_event) => {
            outgoing.abort_pending_server_requests().await;
            respond_to_pending_interrupts(&thread_state, &outgoing).await;

            thread_watch_manager
                .note_turn_interrupted(&conversation_id.to_string())
                .await;
            handle_turn_interrupted(
                conversation_id,
                event_turn_id,
                turn_aborted_event,
                &outgoing,
                &thread_state,
            )
            .await;
        }
        EventMsg::ThreadSettingsApplied(thread_settings_event) => {
            let thread_settings =
                thread_settings_from_core_snapshot(thread_settings_event.thread_settings);
            let changed = {
                let mut state = thread_state.lock().await;
                state.note_thread_settings(thread_settings.clone())
            };
            if changed {
                outgoing
                    .send_server_notification(ServerNotification::ThreadSettingsUpdated(
                        ThreadSettingsUpdatedNotification {
                            thread_id: conversation_id.to_string(),
                            thread_settings,
                        },
                    ))
                    .await;
            }
        }
        EventMsg::ShutdownComplete => {
            thread_watch_manager
                .note_thread_shutdown(&conversation_id.to_string())
                .await;
        }

        _ => {}
    }
}

struct TurnCompletionMetadata {
    status: TurnStatus,
    error: Option<TurnError>,
    started_at: Option<i64>,
    completed_at: Option<i64>,
    duration_ms: Option<i64>,
}

async fn emit_turn_completed_with_status(
    conversation_id: ThreadId,
    event_turn_id: String,
    turn_completion_metadata: TurnCompletionMetadata,
    outgoing: &ThreadScopedOutgoingMessageSender,
) {
    let notification = TurnCompletedNotification {
        thread_id: conversation_id.to_string(),
        turn: Turn {
            id: event_turn_id,
            items: vec![],
            items_view: TurnItemsView::NotLoaded,
            error: turn_completion_metadata.error,
            status: turn_completion_metadata.status,
            started_at: turn_completion_metadata.started_at,
            completed_at: turn_completion_metadata.completed_at,
            duration_ms: turn_completion_metadata.duration_ms,
        },
    };
    outgoing
        .send_server_notification(ServerNotification::TurnCompleted(notification))
        .await;
}

async fn find_and_remove_turn_summary(
    _conversation_id: ThreadId,
    thread_state: &Arc<Mutex<ThreadState>>,
) -> TurnSummary {
    let mut state = thread_state.lock().await;
    std::mem::take(&mut state.turn_summary)
}

async fn handle_turn_complete(
    conversation_id: ThreadId,
    event_turn_id: String,
    turn_complete_event: TurnCompleteEvent,
    outgoing: &ThreadScopedOutgoingMessageSender,
    thread_state: &Arc<Mutex<ThreadState>>,
) {
    let turn_summary = find_and_remove_turn_summary(conversation_id, thread_state).await;

    let (status, error) = match turn_summary.last_error {
        Some(error) => (TurnStatus::Failed, Some(error)),
        None => (TurnStatus::Completed, None),
    };

    emit_turn_completed_with_status(
        conversation_id,
        event_turn_id,
        TurnCompletionMetadata {
            status,
            error,
            started_at: turn_summary.started_at,
            completed_at: turn_complete_event.completed_at,
            duration_ms: turn_complete_event.duration_ms,
        },
        outgoing,
    )
    .await;
}

async fn handle_turn_interrupted(
    conversation_id: ThreadId,
    event_turn_id: String,
    turn_aborted_event: TurnAbortedEvent,
    outgoing: &ThreadScopedOutgoingMessageSender,
    thread_state: &Arc<Mutex<ThreadState>>,
) {
    let turn_summary = find_and_remove_turn_summary(conversation_id, thread_state).await;

    emit_turn_completed_with_status(
        conversation_id,
        event_turn_id,
        TurnCompletionMetadata {
            status: TurnStatus::Interrupted,
            error: None,
            started_at: turn_summary.started_at,
            completed_at: turn_aborted_event.completed_at,
            duration_ms: turn_aborted_event.duration_ms,
        },
        outgoing,
    )
    .await;
}

async fn respond_to_pending_interrupts(
    thread_state: &Arc<Mutex<ThreadState>>,
    outgoing: &ThreadScopedOutgoingMessageSender,
) {
    let pending = {
        let mut state = thread_state.lock().await;
        std::mem::take(&mut state.pending_interrupts)
    };

    for request_id in pending {
        outgoing
            .send_response(request_id, TurnInterruptResponse {})
            .await;
    }
}

async fn handle_token_count_event(
    conversation_id: ThreadId,
    turn_id: String,
    token_count_event: TokenCountEvent,
    outgoing: &ThreadScopedOutgoingMessageSender,
) {
    let TokenCountEvent { info, rate_limits } = token_count_event;
    if let Some(token_usage) = info.map(ThreadTokenUsage::from) {
        let notification = ThreadTokenUsageUpdatedNotification {
            thread_id: conversation_id.to_string(),
            turn_id,
            token_usage,
        };
        outgoing
            .send_server_notification(ServerNotification::ThreadTokenUsageUpdated(notification))
            .await;
    }
    if let Some(rate_limits) = rate_limits {
        outgoing
            .send_server_notification(ServerNotification::AccountRateLimitsUpdated(
                AccountRateLimitsUpdatedNotification {
                    rate_limits: rate_limits.into(),
                },
            ))
            .await;
    }
}

async fn handle_error(
    _conversation_id: ThreadId,
    error: TurnError,
    thread_state: &Arc<Mutex<ThreadState>>,
) {
    let mut state = thread_state.lock().await;
    state.turn_summary.last_error = Some(error);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CHANNEL_CAPACITY;
    use crate::outgoing_message::ConnectionId;
    use crate::outgoing_message::OutgoingEnvelope;
    use crate::outgoing_message::OutgoingMessage;
    use crate::outgoing_message::OutgoingMessageSender;
    use anyhow::Result;
    use anyhow::anyhow;
    use anyhow::bail;
    use codex_protocol::protocol::CreditsSnapshot;
    use codex_protocol::protocol::EventMsg;
    use codex_protocol::protocol::RateLimitSnapshot;
    use codex_protocol::protocol::RateLimitWindow;
    use codex_protocol::protocol::TokenUsage;
    use codex_protocol::protocol::TokenUsageInfo;
    use pretty_assertions::assert_eq;
    use tokio::sync::Mutex;
    use tokio::sync::mpsc;

    fn new_thread_state() -> Arc<Mutex<ThreadState>> {
        Arc::new(Mutex::new(ThreadState::default()))
    }

    const TEST_TURN_COMPLETED_AT: i64 = 1_716_000_456;
    const TEST_TURN_DURATION_MS: i64 = 1_234;

    async fn recv_broadcast_message(
        rx: &mut mpsc::Receiver<OutgoingEnvelope>,
    ) -> Result<OutgoingMessage> {
        let envelope = rx
            .recv()
            .await
            .ok_or_else(|| anyhow!("should send one message"))?;
        match envelope {
            OutgoingEnvelope::Broadcast { message } => Ok(message),
            OutgoingEnvelope::ToConnection { message, .. } => Ok(message),
        }
    }

    fn turn_complete_event(turn_id: &str) -> TurnCompleteEvent {
        TurnCompleteEvent {
            turn_id: turn_id.to_string(),
            last_agent_message: None,
            completed_at: Some(TEST_TURN_COMPLETED_AT),
            duration_ms: Some(TEST_TURN_DURATION_MS),
            time_to_first_token_ms: None,
        }
    }

    fn turn_aborted_event(turn_id: &str) -> TurnAbortedEvent {
        TurnAbortedEvent {
            turn_id: Some(turn_id.to_string()),
            reason: codex_protocol::protocol::TurnAbortReason::Interrupted,
            completed_at: Some(TEST_TURN_COMPLETED_AT),
            duration_ms: Some(TEST_TURN_DURATION_MS),
        }
    }

    #[tokio::test]
    async fn test_handle_error_records_message() -> Result<()> {
        let conversation_id = ThreadId::new();
        let thread_state = new_thread_state();

        handle_error(
            conversation_id,
            TurnError {
                message: "boom".to_string(),
                codex_error_info: Some(V2CodexErrorInfo::InternalServerError),
                additional_details: None,
            },
            &thread_state,
        )
        .await;

        let turn_summary = find_and_remove_turn_summary(conversation_id, &thread_state).await;
        assert_eq!(
            turn_summary.last_error,
            Some(TurnError {
                message: "boom".to_string(),
                codex_error_info: Some(V2CodexErrorInfo::InternalServerError),
                additional_details: None,
            })
        );
        Ok(())
    }

    #[tokio::test]
    async fn turn_started_omits_active_snapshot_items() -> Result<()> {
        let conversation_id = ThreadId::new();
        let thread_state = new_thread_state();
        {
            let mut state = thread_state.lock().await;
            state.track_current_turn_event(
                "turn-1",
                &EventMsg::TurnStarted(codex_protocol::protocol::TurnStartedEvent {
                    turn_id: "turn-1".to_string(),
                    trace_id: None,
                    started_at: Some(42),
                    model_context_window: None,
                }),
            );
        }
        let thread_watch_manager = ThreadWatchManager::new();
        let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);
        let outgoing = Arc::new(OutgoingMessageSender::new(tx));
        let outgoing = ThreadScopedOutgoingMessageSender::new(
            outgoing,
            vec![ConnectionId(1)],
            conversation_id,
        );

        apply_bespoke_event_handling(
            Event {
                id: "turn-1".to_string(),
                msg: EventMsg::TurnStarted(codex_protocol::protocol::TurnStartedEvent {
                    turn_id: "turn-1".to_string(),
                    trace_id: None,
                    started_at: Some(42),
                    model_context_window: None,
                }),
            },
            conversation_id,
            outgoing,
            thread_state,
            thread_watch_manager,
        )
        .await;

        let msg = recv_broadcast_message(&mut rx).await?;
        match msg {
            OutgoingMessage::AppServerNotification(ServerNotification::TurnStarted(n)) => {
                assert_eq!(n.turn.id, "turn-1");
                assert_eq!(n.turn.items_view, TurnItemsView::NotLoaded);
                assert!(n.turn.items.is_empty());
            }
            other => bail!("unexpected message: {other:?}"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn test_handle_turn_complete_emits_completed_without_error() -> Result<()> {
        let conversation_id = ThreadId::new();
        let event_turn_id = "complete1".to_string();
        let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);
        let outgoing = Arc::new(OutgoingMessageSender::new(tx));
        let outgoing = ThreadScopedOutgoingMessageSender::new(
            outgoing,
            vec![ConnectionId(1)],
            ThreadId::new(),
        );
        let thread_state = new_thread_state();
        {
            let mut state = thread_state.lock().await;
            state.track_current_turn_event(
                &event_turn_id,
                &EventMsg::TurnStarted(codex_protocol::protocol::TurnStartedEvent {
                    turn_id: event_turn_id.clone(),
                    trace_id: None,
                    started_at: Some(42),
                    model_context_window: None,
                }),
            );
            state.track_current_turn_event(
                &event_turn_id,
                &EventMsg::TurnComplete(turn_complete_event(&event_turn_id)),
            );
        }

        handle_turn_complete(
            conversation_id,
            event_turn_id.clone(),
            turn_complete_event(&event_turn_id),
            &outgoing,
            &thread_state,
        )
        .await;

        let msg = recv_broadcast_message(&mut rx).await?;
        match msg {
            OutgoingMessage::AppServerNotification(ServerNotification::TurnCompleted(n)) => {
                assert_eq!(n.turn.id, event_turn_id);
                assert_eq!(n.turn.status, TurnStatus::Completed);
                assert_eq!(n.turn.items_view, TurnItemsView::NotLoaded);
                assert!(n.turn.items.is_empty());
                assert_eq!(n.turn.error, None);
                assert_eq!(n.turn.started_at, Some(42));
                assert_eq!(n.turn.completed_at, Some(TEST_TURN_COMPLETED_AT));
                assert_eq!(n.turn.duration_ms, Some(TEST_TURN_DURATION_MS));
            }
            other => bail!("unexpected message: {other:?}"),
        }
        assert!(rx.try_recv().is_err(), "no extra messages expected");
        Ok(())
    }

    #[tokio::test]
    async fn test_handle_turn_interrupted_emits_interrupted_with_error() -> Result<()> {
        let conversation_id = ThreadId::new();
        let event_turn_id = "interrupt1".to_string();
        let thread_state = new_thread_state();
        handle_error(
            conversation_id,
            TurnError {
                message: "oops".to_string(),
                codex_error_info: None,
                additional_details: None,
            },
            &thread_state,
        )
        .await;
        let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);
        let outgoing = Arc::new(OutgoingMessageSender::new(tx));
        let outgoing = ThreadScopedOutgoingMessageSender::new(
            outgoing,
            vec![ConnectionId(1)],
            ThreadId::new(),
        );

        handle_turn_interrupted(
            conversation_id,
            event_turn_id.clone(),
            turn_aborted_event(&event_turn_id),
            &outgoing,
            &thread_state,
        )
        .await;

        let msg = recv_broadcast_message(&mut rx).await?;
        match msg {
            OutgoingMessage::AppServerNotification(ServerNotification::TurnCompleted(n)) => {
                assert_eq!(n.turn.id, event_turn_id);
                assert_eq!(n.turn.status, TurnStatus::Interrupted);
                assert_eq!(n.turn.error, None);
                assert_eq!(n.turn.completed_at, Some(TEST_TURN_COMPLETED_AT));
                assert_eq!(n.turn.duration_ms, Some(TEST_TURN_DURATION_MS));
            }
            other => bail!("unexpected message: {other:?}"),
        }
        assert!(rx.try_recv().is_err(), "no extra messages expected");
        Ok(())
    }

    #[tokio::test]
    async fn test_handle_turn_complete_emits_failed_with_error() -> Result<()> {
        let conversation_id = ThreadId::new();
        let event_turn_id = "complete_err1".to_string();
        let thread_state = new_thread_state();
        handle_error(
            conversation_id,
            TurnError {
                message: "bad".to_string(),
                codex_error_info: Some(V2CodexErrorInfo::Other),
                additional_details: None,
            },
            &thread_state,
        )
        .await;
        let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);
        let outgoing = Arc::new(OutgoingMessageSender::new(tx));
        let outgoing = ThreadScopedOutgoingMessageSender::new(
            outgoing,
            vec![ConnectionId(1)],
            ThreadId::new(),
        );

        handle_turn_complete(
            conversation_id,
            event_turn_id.clone(),
            turn_complete_event(&event_turn_id),
            &outgoing,
            &thread_state,
        )
        .await;

        let msg = recv_broadcast_message(&mut rx).await?;
        match msg {
            OutgoingMessage::AppServerNotification(ServerNotification::TurnCompleted(n)) => {
                assert_eq!(n.turn.id, event_turn_id);
                assert_eq!(n.turn.status, TurnStatus::Failed);
                assert_eq!(
                    n.turn.error,
                    Some(TurnError {
                        message: "bad".to_string(),
                        codex_error_info: Some(V2CodexErrorInfo::Other),
                        additional_details: None,
                    })
                );
                assert_eq!(n.turn.completed_at, Some(TEST_TURN_COMPLETED_AT));
                assert_eq!(n.turn.duration_ms, Some(TEST_TURN_DURATION_MS));
            }
            other => bail!("unexpected message: {other:?}"),
        }
        assert!(rx.try_recv().is_err(), "no extra messages expected");
        Ok(())
    }

    #[tokio::test]
    async fn test_handle_token_count_event_emits_usage_and_rate_limits() -> Result<()> {
        let conversation_id = ThreadId::new();
        let turn_id = "turn-123".to_string();
        let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);
        let outgoing = Arc::new(OutgoingMessageSender::new(tx));
        let outgoing = ThreadScopedOutgoingMessageSender::new(
            outgoing,
            vec![ConnectionId(1)],
            ThreadId::new(),
        );

        let info = TokenUsageInfo {
            total_token_usage: TokenUsage {
                input_tokens: 100,
                cached_input_tokens: 25,
                output_tokens: 50,
                reasoning_output_tokens: 9,
                total_tokens: 200,
            },
            last_token_usage: TokenUsage {
                input_tokens: 10,
                cached_input_tokens: 5,
                output_tokens: 7,
                reasoning_output_tokens: 1,
                total_tokens: 23,
            },
            model_context_window: Some(4096),
        };
        let rate_limits = RateLimitSnapshot {
            limit_id: Some("codex".to_string()),
            limit_name: None,
            primary: Some(RateLimitWindow {
                used_percent: 42.5,
                window_minutes: Some(15),
                resets_at: Some(1700000000),
            }),
            secondary: None,
            credits: Some(CreditsSnapshot {
                has_credits: true,
                unlimited: false,
                balance: Some("5".to_string()),
            }),
            plan_type: None,
            rate_limit_reached_type: None,
        };

        handle_token_count_event(
            conversation_id,
            turn_id.clone(),
            TokenCountEvent {
                info: Some(info),
                rate_limits: Some(rate_limits),
            },
            &outgoing,
        )
        .await;

        let first = recv_broadcast_message(&mut rx).await?;
        match first {
            OutgoingMessage::AppServerNotification(
                ServerNotification::ThreadTokenUsageUpdated(payload),
            ) => {
                assert_eq!(payload.thread_id, conversation_id.to_string());
                assert_eq!(payload.turn_id, turn_id);
                let usage = payload.token_usage;
                assert_eq!(usage.total.total_tokens, 200);
                assert_eq!(usage.total.cached_input_tokens, 25);
                assert_eq!(usage.last.output_tokens, 7);
                assert_eq!(usage.model_context_window, Some(4096));
            }
            other => bail!("unexpected notification: {other:?}"),
        }

        let second = recv_broadcast_message(&mut rx).await?;
        match second {
            OutgoingMessage::AppServerNotification(
                ServerNotification::AccountRateLimitsUpdated(payload),
            ) => {
                assert_eq!(payload.rate_limits.limit_id.as_deref(), Some("codex"));
                assert_eq!(payload.rate_limits.limit_name, None);
                assert!(payload.rate_limits.primary.is_some());
                assert!(payload.rate_limits.credits.is_some());
            }
            other => bail!("unexpected notification: {other:?}"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn test_handle_token_count_event_without_usage_info() -> Result<()> {
        let conversation_id = ThreadId::new();
        let turn_id = "turn-456".to_string();
        let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);
        let outgoing = Arc::new(OutgoingMessageSender::new(tx));
        let outgoing = ThreadScopedOutgoingMessageSender::new(
            outgoing,
            vec![ConnectionId(1)],
            ThreadId::new(),
        );

        handle_token_count_event(
            conversation_id,
            turn_id.clone(),
            TokenCountEvent {
                info: None,
                rate_limits: None,
            },
            &outgoing,
        )
        .await;

        assert!(
            rx.try_recv().is_err(),
            "no notifications should be emitted when token usage info is absent"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_handle_turn_complete_emits_error_multiple_turns() -> Result<()> {
        let conversation_a = ThreadId::new();
        let conversation_b = ThreadId::new();
        let thread_state = new_thread_state();

        let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);
        let outgoing = Arc::new(OutgoingMessageSender::new(tx));
        let outgoing = ThreadScopedOutgoingMessageSender::new(
            outgoing,
            vec![ConnectionId(1)],
            ThreadId::new(),
        );

        let a_turn1 = "a_turn1".to_string();
        handle_error(
            conversation_a,
            TurnError {
                message: "a1".to_string(),
                codex_error_info: Some(V2CodexErrorInfo::BadRequest),
                additional_details: None,
            },
            &thread_state,
        )
        .await;
        handle_turn_complete(
            conversation_a,
            a_turn1.clone(),
            turn_complete_event(&a_turn1),
            &outgoing,
            &thread_state,
        )
        .await;

        let b_turn1 = "b_turn1".to_string();
        handle_error(
            conversation_b,
            TurnError {
                message: "b1".to_string(),
                codex_error_info: None,
                additional_details: None,
            },
            &thread_state,
        )
        .await;
        handle_turn_complete(
            conversation_b,
            b_turn1.clone(),
            turn_complete_event(&b_turn1),
            &outgoing,
            &thread_state,
        )
        .await;

        let a_turn2 = "a_turn2".to_string();
        handle_turn_complete(
            conversation_a,
            a_turn2.clone(),
            turn_complete_event(&a_turn2),
            &outgoing,
            &thread_state,
        )
        .await;

        let msg = recv_broadcast_message(&mut rx).await?;
        match msg {
            OutgoingMessage::AppServerNotification(ServerNotification::TurnCompleted(n)) => {
                assert_eq!(n.turn.id, a_turn1);
                assert_eq!(n.turn.status, TurnStatus::Failed);
                assert_eq!(
                    n.turn.error,
                    Some(TurnError {
                        message: "a1".to_string(),
                        codex_error_info: Some(V2CodexErrorInfo::BadRequest),
                        additional_details: None,
                    })
                );
            }
            other => bail!("unexpected message: {other:?}"),
        }

        let msg = recv_broadcast_message(&mut rx).await?;
        match msg {
            OutgoingMessage::AppServerNotification(ServerNotification::TurnCompleted(n)) => {
                assert_eq!(n.turn.id, b_turn1);
                assert_eq!(n.turn.status, TurnStatus::Failed);
                assert_eq!(
                    n.turn.error,
                    Some(TurnError {
                        message: "b1".to_string(),
                        codex_error_info: None,
                        additional_details: None,
                    })
                );
            }
            other => bail!("unexpected message: {other:?}"),
        }

        let msg = recv_broadcast_message(&mut rx).await?;
        match msg {
            OutgoingMessage::AppServerNotification(ServerNotification::TurnCompleted(n)) => {
                assert_eq!(n.turn.id, a_turn2);
                assert_eq!(n.turn.status, TurnStatus::Completed);
                assert_eq!(n.turn.error, None);
            }
            other => bail!("unexpected message: {other:?}"),
        }

        assert!(rx.try_recv().is_err(), "no extra messages expected");
        Ok(())
    }
}
