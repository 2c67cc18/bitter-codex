use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::Notify;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

use crate::session::TurnInputQueue;
use crate::session::turn_context::TurnContext;
use crate::tasks::AnySessionTask;
use codex_protocol::dynamic_tools::DynamicToolResponse;
use codex_protocol::protocol::TokenUsage;

pub(crate) struct ActiveTurn {
    pub(crate) task: Option<RunningTask>,
    pub(crate) turn_state: Arc<Mutex<TurnState>>,
}

impl Default for ActiveTurn {
    fn default() -> Self {
        Self {
            task: None,
            turn_state: Arc::new(Mutex::new(TurnState::default())),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TaskKind {
    Regular,
    Compact,
}

pub(crate) struct RunningTask {
    pub(crate) done: Arc<Notify>,
    pub(crate) kind: TaskKind,
    pub(crate) task: Arc<dyn AnySessionTask>,
    pub(crate) cancellation_token: CancellationToken,
    pub(crate) handle: AbortOnDropHandle<()>,
    pub(crate) turn_context: Arc<TurnContext>,

    pub(crate) _timer: Option<codex_otel::Timer>,
}

#[derive(Default)]
pub(crate) struct TurnState {
    pending_dynamic_tools: HashMap<String, oneshot::Sender<DynamicToolResponse>>,
    pub(crate) pending_input: TurnInputQueue,
    pub(crate) tool_calls: u64,
    pub(crate) token_usage_at_turn_start: TokenUsage,
}

impl TurnState {
    pub(crate) fn clear_pending_waiters(&mut self) {
        self.pending_dynamic_tools.clear();
    }

    pub(crate) fn insert_pending_dynamic_tool(
        &mut self,
        key: String,
        tx: oneshot::Sender<DynamicToolResponse>,
    ) -> Option<oneshot::Sender<DynamicToolResponse>> {
        self.pending_dynamic_tools.insert(key, tx)
    }

    pub(crate) fn remove_pending_dynamic_tool(
        &mut self,
        key: &str,
    ) -> Option<oneshot::Sender<DynamicToolResponse>> {
        self.pending_dynamic_tools.remove(key)
    }
}
