#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::time::Duration;
use std::time::Instant;

use async_channel::Sender;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tokio::io::BufReader;
use tokio::process::Child;
use tokio_util::sync::CancellationToken;

use crate::spawn::SpawnChildRequest;
use crate::spawn::StdioPolicy;
use crate::spawn::spawn_child_async;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result;
use codex_protocol::exec_output::ExecToolCallOutput;
use codex_protocol::exec_output::StreamOutput;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ExecCommandOutputDeltaEvent;
use codex_protocol::protocol::ExecOutputStream;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_pty::DEFAULT_OUTPUT_BYTES_CAP;
use codex_utils_pty::process_group::kill_child_process_group;

pub const DEFAULT_EXEC_COMMAND_TIMEOUT_MS: u64 = 10_000;

const TIMEOUT_CODE: i32 = 64;
const EXIT_CODE_SIGNAL_BASE: i32 = 128;
const EXEC_TIMEOUT_EXIT_CODE: i32 = 124;

const READ_CHUNK_SIZE: usize = 8192;
const AGGREGATE_BUFFER_INITIAL_CAPACITY: usize = 8 * 1024;
const EXEC_OUTPUT_MAX_BYTES: usize = DEFAULT_OUTPUT_BYTES_CAP;

pub(crate) const MAX_EXEC_OUTPUT_DELTAS_PER_CALL: usize = 10_000;
pub const IO_DRAIN_TIMEOUT_MS: u64 = 2_000;

#[derive(Debug)]
pub struct ExecParams {
    pub command: Vec<String>,
    pub cwd: AbsolutePathBuf,
    pub expiration: ExecExpiration,
    pub capture_policy: ExecCapturePolicy,
    pub env: HashMap<String, String>,
    pub justification: Option<String>,
    pub arg0: Option<String>,
}

#[derive(Debug)]
pub struct ExecRequest {
    pub command: Vec<String>,
    pub cwd: AbsolutePathBuf,
    pub env: HashMap<String, String>,
    pub expiration: ExecExpiration,
    pub capture_policy: ExecCapturePolicy,
    pub arg0: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExecCapturePolicy {
    #[default]
    ShellTool,

    FullBuffer,
}

#[derive(Clone, Debug)]
pub enum ExecExpiration {
    Timeout(Duration),
    DefaultTimeout,
    Cancellation(CancellationToken),
    TimeoutOrCancellation {
        timeout: Duration,
        cancellation: CancellationToken,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecExpirationOutcome {
    TimedOut,

    Cancelled,
}

impl From<Option<u64>> for ExecExpiration {
    fn from(timeout_ms: Option<u64>) -> Self {
        timeout_ms.map_or(ExecExpiration::DefaultTimeout, |timeout_ms| {
            ExecExpiration::Timeout(Duration::from_millis(timeout_ms))
        })
    }
}
impl From<u64> for ExecExpiration {
    fn from(timeout_ms: u64) -> Self {
        ExecExpiration::Timeout(Duration::from_millis(timeout_ms))
    }
}

impl ExecExpiration {
    pub async fn wait_with_outcome(self) -> ExecExpirationOutcome {
        match self {
            ExecExpiration::Timeout(duration) => {
                tokio::time::sleep(duration).await;
                ExecExpirationOutcome::TimedOut
            }
            ExecExpiration::DefaultTimeout => {
                tokio::time::sleep(Duration::from_millis(DEFAULT_EXEC_COMMAND_TIMEOUT_MS)).await;
                ExecExpirationOutcome::TimedOut
            }
            ExecExpiration::Cancellation(cancel) => {
                cancel.cancelled().await;
                ExecExpirationOutcome::Cancelled
            }
            ExecExpiration::TimeoutOrCancellation {
                timeout,
                cancellation,
            } => {
                tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => ExecExpirationOutcome::Cancelled,
                    _ = tokio::time::sleep(timeout) => ExecExpirationOutcome::TimedOut,
                }
            }
        }
    }

    pub(crate) fn timeout_ms(&self) -> Option<u64> {
        match self {
            ExecExpiration::Timeout(duration) => Some(duration.as_millis() as u64),
            ExecExpiration::DefaultTimeout => Some(DEFAULT_EXEC_COMMAND_TIMEOUT_MS),
            ExecExpiration::Cancellation(_) => None,
            ExecExpiration::TimeoutOrCancellation { timeout, .. } => {
                Some(timeout.as_millis() as u64)
            }
        }
    }

}


impl ExecCapturePolicy {
    fn retained_bytes_cap(self) -> Option<usize> {
        match self {
            Self::ShellTool => Some(EXEC_OUTPUT_MAX_BYTES),
            Self::FullBuffer => None,
        }
    }

    fn io_drain_timeout(self) -> Duration {
        Duration::from_millis(IO_DRAIN_TIMEOUT_MS)
    }

    fn uses_expiration(self) -> bool {
        match self {
            Self::ShellTool => true,
            Self::FullBuffer => false,
        }
    }
}

#[derive(Clone)]
pub struct StdoutStream {
    pub sub_id: String,
    pub call_id: String,
    pub tx_event: Sender<Event>,
}

#[allow(clippy::too_many_arguments)]
pub async fn process_exec_tool_call(
    params: ExecParams,
    stdout_stream: Option<StdoutStream>,
) -> Result<ExecToolCallOutput> {
    let start = Instant::now();
    let raw_output_result = exec(params, stdout_stream, None).await;
    let duration = start.elapsed();
    finalize_exec_result(raw_output_result, duration)
}

pub fn build_exec_request(params: ExecParams) -> Result<ExecRequest> {
    let ExecParams {
        command,
        cwd,
        env,
        expiration,
        capture_policy,
        arg0,
        justification: _,
    } = params;

    if command.is_empty() {
        return Err(CodexErr::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "command args are empty",
        )));
    }

    Ok(ExecRequest {
        command,
        cwd,
        env,
        expiration,
        capture_policy,
        arg0,
    })
}


async fn exec(
    params: ExecParams,
    stdout_stream: Option<StdoutStream>,
    after_spawn: Option<Box<dyn FnOnce() + Send>>,
) -> Result<RawExecToolCallOutput> {
    let ExecParams {
        command,
        cwd,
        env,
        arg0,
        expiration,
        capture_policy,
        justification: _,
    } = params;

    let (program, args) = command.split_first().ok_or_else(|| {
        CodexErr::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "command args are empty",
        ))
    })?;
    let arg0_ref = arg0.as_deref();
    let child = spawn_child_async(SpawnChildRequest {
        program: PathBuf::from(program),
        args: args.into(),
        arg0: arg0_ref,
        cwd,
        stdio_policy: StdioPolicy::RedirectForShellTool,
        env,
    })
    .await?;
    if let Some(after_spawn) = after_spawn {
        after_spawn();
    }
    consume_output(child, expiration, capture_policy, stdout_stream).await
}

fn finalize_exec_result(
    raw_output_result: std::result::Result<RawExecToolCallOutput, CodexErr>,
    duration: Duration,
) -> Result<ExecToolCallOutput> {
    match raw_output_result {
        Ok(raw_output) => {
            #[allow(unused_mut)]
            let mut timed_out = raw_output.timed_out;

            #[cfg(target_family = "unix")]
            {
                if let Some(signal) = raw_output.exit_status.signal() {
                    if signal == TIMEOUT_CODE {
                        timed_out = true;
                    } else {
                        return Err(CodexErr::Fatal(format!(
                            "process terminated by signal {signal}"
                        )));
                    }
                }
            }

            let mut exit_code = raw_output.exit_status.code().unwrap_or(-1);
            if timed_out {
                exit_code = EXEC_TIMEOUT_EXIT_CODE;
            }

            let stdout = raw_output.stdout.from_utf8_lossy();
            let stderr = raw_output.stderr.from_utf8_lossy();
            let aggregated_output = raw_output.aggregated_output.from_utf8_lossy();
            let exec_output = ExecToolCallOutput {
                exit_code,
                stdout,
                stderr,
                aggregated_output,
                duration,
                timed_out,
            };

            Ok(exec_output)
        }
        Err(err) => {
            tracing::error!("exec error: {err}");
            Err(err)
        }
    }
}

#[derive(Debug)]
struct RawExecToolCallOutput {
    pub exit_status: ExitStatus,
    pub stdout: StreamOutput<Vec<u8>>,
    pub stderr: StreamOutput<Vec<u8>>,
    pub aggregated_output: StreamOutput<Vec<u8>>,
    pub timed_out: bool,
}

#[inline]
fn append_capped(dst: &mut Vec<u8>, src: &[u8], max_bytes: usize) {
    if dst.len() >= max_bytes {
        return;
    }
    let remaining = max_bytes.saturating_sub(dst.len());
    let take = remaining.min(src.len());
    dst.extend_from_slice(&src[..take]);
}

fn aggregate_output(
    stdout: &StreamOutput<Vec<u8>>,
    stderr: &StreamOutput<Vec<u8>>,
    max_bytes: Option<usize>,
) -> StreamOutput<Vec<u8>> {
    let Some(max_bytes) = max_bytes else {
        let total_len = stdout.text.len().saturating_add(stderr.text.len());
        let mut aggregated = Vec::with_capacity(total_len);
        aggregated.extend_from_slice(&stdout.text);
        aggregated.extend_from_slice(&stderr.text);
        return StreamOutput {
            text: aggregated,
            truncated_after_lines: None,
        };
    };

    let total_len = stdout.text.len().saturating_add(stderr.text.len());
    let mut aggregated = Vec::with_capacity(total_len.min(max_bytes));

    if total_len <= max_bytes {
        aggregated.extend_from_slice(&stdout.text);
        aggregated.extend_from_slice(&stderr.text);
        return StreamOutput {
            text: aggregated,
            truncated_after_lines: None,
        };
    }

    let want_stdout = stdout.text.len().min(max_bytes / 3);
    let want_stderr = stderr.text.len();
    let stderr_take = want_stderr.min(max_bytes.saturating_sub(want_stdout));
    let remaining = max_bytes.saturating_sub(want_stdout + stderr_take);
    let stdout_take = want_stdout + remaining.min(stdout.text.len().saturating_sub(want_stdout));

    aggregated.extend_from_slice(&stdout.text[..stdout_take]);
    aggregated.extend_from_slice(&stderr.text[..stderr_take]);

    StreamOutput {
        text: aggregated,
        truncated_after_lines: None,
    }
}

async fn consume_output(
    mut child: Child,
    expiration: ExecExpiration,
    capture_policy: ExecCapturePolicy,
    stdout_stream: Option<StdoutStream>,
) -> Result<RawExecToolCallOutput> {
    let stdout_reader = child.stdout.take().ok_or_else(|| {
        CodexErr::Io(io::Error::other(
            "stdout pipe was unexpectedly not available",
        ))
    })?;
    let stderr_reader = child.stderr.take().ok_or_else(|| {
        CodexErr::Io(io::Error::other(
            "stderr pipe was unexpectedly not available",
        ))
    })?;

    let retained_bytes_cap = capture_policy.retained_bytes_cap();
    let stdout_handle = tokio::spawn(read_output(
        BufReader::new(stdout_reader),
        stdout_stream.clone(),
        false,
        retained_bytes_cap,
    ));
    let stderr_handle = tokio::spawn(read_output(
        BufReader::new(stderr_reader),
        stdout_stream.clone(),
        true,
        retained_bytes_cap,
    ));

    let expiration_wait = async {
        if capture_policy.uses_expiration() {
            Some(expiration.wait_with_outcome().await)
        } else {
            std::future::pending::<Option<ExecExpirationOutcome>>().await
        }
    };
    tokio::pin!(expiration_wait);
    let (exit_status, timed_out) = tokio::select! {
        status_result = child.wait() => {
            let exit_status = status_result?;
            (exit_status, false)
        }
        outcome = &mut expiration_wait => {
            kill_child_process_group(&mut child)?;
            child.start_kill()?;
            let timed_out = matches!(outcome, Some(ExecExpirationOutcome::TimedOut));
            let exit_status = if timed_out {
                synthetic_exit_status(EXIT_CODE_SIGNAL_BASE + TIMEOUT_CODE)
            } else {
                synthetic_exit_status(1)
            };
            (exit_status, timed_out)
        }
        _ = tokio::signal::ctrl_c() => {
            kill_child_process_group(&mut child)?;
            child.start_kill()?;
            (synthetic_exit_status(1), false)
        }
    };

    use tokio::task::JoinHandle;

    async fn await_output(
        handle: &mut JoinHandle<std::io::Result<StreamOutput<Vec<u8>>>>,
        timeout: Duration,
    ) -> std::io::Result<StreamOutput<Vec<u8>>> {
        match tokio::time::timeout(timeout, &mut *handle).await {
            Ok(join_res) => match join_res {
                Ok(io_res) => io_res,
                Err(join_err) => Err(std::io::Error::other(join_err)),
            },
            Err(_elapsed) => {
                handle.abort();
                Ok(StreamOutput {
                    text: Vec::new(),
                    truncated_after_lines: None,
                })
            }
        }
    }

    let mut stdout_handle = stdout_handle;
    let mut stderr_handle = stderr_handle;

    let stdout = await_output(&mut stdout_handle, capture_policy.io_drain_timeout()).await?;
    let stderr = await_output(&mut stderr_handle, capture_policy.io_drain_timeout()).await?;
    let aggregated_output = aggregate_output(&stdout, &stderr, Some(EXEC_OUTPUT_MAX_BYTES));

    Ok(RawExecToolCallOutput {
        exit_status,
        stdout,
        stderr,
        aggregated_output,
        timed_out,
    })
}

async fn read_output<R: AsyncRead + Unpin + Send + 'static>(
    mut reader: R,
    stream: Option<StdoutStream>,
    is_stderr: bool,
    max_bytes: Option<usize>,
) -> io::Result<StreamOutput<Vec<u8>>> {
    let mut buf = Vec::with_capacity(
        max_bytes.map_or(AGGREGATE_BUFFER_INITIAL_CAPACITY, |max_bytes| {
            AGGREGATE_BUFFER_INITIAL_CAPACITY.min(max_bytes)
        }),
    );
    let mut tmp = [0u8; READ_CHUNK_SIZE];
    let mut emitted_deltas: usize = 0;

    loop {
        let n = reader.read(&mut tmp).await?;
        if n == 0 {
            break;
        }

        if let Some(stream) = &stream
            && emitted_deltas < MAX_EXEC_OUTPUT_DELTAS_PER_CALL
        {
            let chunk = tmp[..n].to_vec();
            let msg = EventMsg::ExecCommandOutputDelta(ExecCommandOutputDeltaEvent {
                call_id: stream.call_id.clone(),
                stream: if is_stderr {
                    ExecOutputStream::Stderr
                } else {
                    ExecOutputStream::Stdout
                },
                chunk,
            });
            let event = Event {
                id: stream.sub_id.clone(),
                msg,
            };
            #[allow(clippy::let_unit_value)]
            let _ = stream.tx_event.send(event).await;
            emitted_deltas += 1;
        }

        if let Some(max_bytes) = max_bytes {
            append_capped(&mut buf, &tmp[..n], max_bytes);
        } else {
            buf.extend_from_slice(&tmp[..n]);
        }
    }

    Ok(StreamOutput {
        text: buf,
        truncated_after_lines: None,
    })
}

fn synthetic_exit_status(code: i32) -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(code << 8)
}
