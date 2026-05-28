#![allow(clippy::module_inception)]

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use tokio::sync::Mutex;
use tokio::sync::Notify;
use tokio::sync::broadcast;
use tokio::sync::oneshot::error::TryRecvError;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;

use codex_utils_pty::ExecCommandSession;
use codex_utils_pty::SpawnedPty;

use super::UnifiedExecError;
use super::head_tail_buffer::HeadTailBuffer;
use super::process_state::ProcessState;

const EARLY_EXIT_GRACE_PERIOD: Duration = Duration::from_millis(150);

pub(crate) trait SpawnLifecycle: std::fmt::Debug + Send + Sync {
    fn inherited_fds(&self) -> Vec<i32> {
        Vec::new()
    }

    fn after_spawn(&mut self) {}
}

pub(crate) type SpawnLifecycleHandle = Box<dyn SpawnLifecycle>;

#[derive(Debug, Default)]

pub(crate) struct NoopSpawnLifecycle;

impl SpawnLifecycle for NoopSpawnLifecycle {}

pub(crate) type OutputBuffer = Arc<Mutex<HeadTailBuffer>>;

pub(crate) struct OutputHandles {
    pub(crate) output_buffer: OutputBuffer,
    pub(crate) output_notify: Arc<Notify>,
    pub(crate) output_closed: Arc<AtomicBool>,
    pub(crate) output_closed_notify: Arc<Notify>,
    pub(crate) cancellation_token: CancellationToken,
}

pub(crate) struct UnifiedExecProcess {
    process_handle: Box<ExecCommandSession>,
    output_tx: broadcast::Sender<Vec<u8>>,
    output_buffer: OutputBuffer,
    output_notify: Arc<Notify>,
    output_closed: Arc<AtomicBool>,
    output_closed_notify: Arc<Notify>,
    cancellation_token: CancellationToken,
    output_drained: Arc<Notify>,
    state_tx: watch::Sender<ProcessState>,
    state_rx: watch::Receiver<ProcessState>,
    output_task: Option<JoinHandle<()>>,
    _spawn_lifecycle: Option<SpawnLifecycleHandle>,
}

impl std::fmt::Debug for UnifiedExecProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnifiedExecProcess")
            .field("has_exited", &self.has_exited())
            .field("exit_code", &self.exit_code())
            .finish_non_exhaustive()
    }
}

impl UnifiedExecProcess {
    fn new(
        process_handle: Box<ExecCommandSession>,
        spawn_lifecycle: Option<SpawnLifecycleHandle>,
    ) -> Self {
        let output_buffer = Arc::new(Mutex::new(HeadTailBuffer::default()));
        let output_notify = Arc::new(Notify::new());
        let output_closed = Arc::new(AtomicBool::new(false));
        let output_closed_notify = Arc::new(Notify::new());
        let cancellation_token = CancellationToken::new();
        let output_drained = Arc::new(Notify::new());
        let (output_tx, _) = broadcast::channel(64);
        let (state_tx, state_rx) = watch::channel(ProcessState::default());

        Self {
            process_handle,
            output_tx,
            output_buffer,
            output_notify,
            output_closed,
            output_closed_notify,
            cancellation_token,
            output_drained,
            state_tx,
            state_rx,
            output_task: None,
            _spawn_lifecycle: spawn_lifecycle,
        }
    }

    pub(super) async fn write(&self, data: &[u8]) -> Result<(), UnifiedExecError> {
        self.process_handle
            .writer_sender()
            .send(data.to_vec())
            .await
            .map_err(|_| UnifiedExecError::WriteToStdin)
    }

    pub(super) fn output_handles(&self) -> OutputHandles {
        OutputHandles {
            output_buffer: Arc::clone(&self.output_buffer),
            output_notify: Arc::clone(&self.output_notify),
            output_closed: Arc::clone(&self.output_closed),
            output_closed_notify: Arc::clone(&self.output_closed_notify),
            cancellation_token: self.cancellation_token.clone(),
        }
    }

    pub(super) fn output_receiver(&self) -> tokio::sync::broadcast::Receiver<Vec<u8>> {
        self.output_tx.subscribe()
    }

    pub(super) fn cancellation_token(&self) -> CancellationToken {
        self.cancellation_token.clone()
    }

    pub(super) fn output_drained_notify(&self) -> Arc<Notify> {
        Arc::clone(&self.output_drained)
    }

    pub(super) fn has_exited(&self) -> bool {
        let state = self.state_rx.borrow().clone();
        state.has_exited || self.process_handle.has_exited()
    }

    pub(super) fn exit_code(&self) -> Option<i32> {
        let state = self.state_rx.borrow().clone();
        state.exit_code.or_else(|| self.process_handle.exit_code())
    }

    pub(super) fn terminate(&self) {
        self.output_closed.store(true, Ordering::Release);
        self.output_closed_notify.notify_waiters();
        self.process_handle.terminate();
        self.cancellation_token.cancel();
        if let Some(output_task) = &self.output_task {
            output_task.abort();
        }
    }

    pub(super) fn fail_and_terminate(&self, message: String) {
        let state = self.state_rx.borrow().clone();
        if state.failure_message.is_none() {
            let _ = self.state_tx.send_replace(state.failed(message));
        }
        self.terminate();
    }

    pub(super) fn failure_message(&self) -> Option<String> {
        self.state_rx.borrow().failure_message.clone()
    }
    pub(super) async fn from_spawned(
        spawned: SpawnedPty,
        spawn_lifecycle: SpawnLifecycleHandle,
    ) -> Result<Self, UnifiedExecError> {
        let SpawnedPty {
            session: process_handle,
            stdout_rx,
            stderr_rx,
            mut exit_rx,
        } = spawned;
        let output_rx = codex_utils_pty::combine_output_receivers(stdout_rx, stderr_rx);
        let mut managed = Self::new(Box::new(process_handle), Some(spawn_lifecycle));
        managed.output_task = Some(Self::spawn_local_output_task(
            output_rx,
            Arc::clone(&managed.output_buffer),
            Arc::clone(&managed.output_notify),
            Arc::clone(&managed.output_closed),
            Arc::clone(&managed.output_closed_notify),
            managed.output_tx.clone(),
        ));

        match exit_rx.try_recv() {
            Ok(exit_code) => {
                managed.signal_exit(Some(exit_code));
                return Ok(managed);
            }
            Err(TryRecvError::Closed) => {
                managed.signal_exit(None);
                return Ok(managed);
            }
            Err(TryRecvError::Empty) => {}
        }

        if let Ok(exit_result) = tokio::time::timeout(EARLY_EXIT_GRACE_PERIOD, &mut exit_rx).await {
            managed.signal_exit(exit_result.ok());
            return Ok(managed);
        }

        tokio::spawn({
            let state_tx = managed.state_tx.clone();
            let cancellation_token = managed.cancellation_token.clone();
            async move {
                let exit_code = exit_rx.await.ok();
                let state = state_tx.borrow().clone();
                let _ = state_tx.send_replace(state.exited(exit_code));
                cancellation_token.cancel();
            }
        });

        Ok(managed)
    }

    fn spawn_local_output_task(
        mut receiver: tokio::sync::broadcast::Receiver<Vec<u8>>,
        buffer: OutputBuffer,
        output_notify: Arc<Notify>,
        output_closed: Arc<AtomicBool>,
        output_closed_notify: Arc<Notify>,
        output_tx: broadcast::Sender<Vec<u8>>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                match receiver.recv().await {
                    Ok(chunk) => {
                        let mut guard = buffer.lock().await;
                        guard.push_chunk(chunk.clone());
                        drop(guard);
                        let _ = output_tx.send(chunk);
                        output_notify.notify_waiters();
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        output_closed.store(true, Ordering::Release);
                        output_closed_notify.notify_waiters();
                        break;
                    }
                };
            }
        })
    }

    fn signal_exit(&self, exit_code: Option<i32>) {
        let state = self.state_rx.borrow().clone();
        let _ = self.state_tx.send_replace(state.exited(exit_code));
        self.cancellation_token.cancel();
    }
}

impl Drop for UnifiedExecProcess {
    fn drop(&mut self) {
        self.terminate();
    }
}
