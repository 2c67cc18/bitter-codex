use super::head_tail_buffer::HeadTailBuffer;
use super::*;
use crate::session::session::Session;
use crate::session::tests::make_session_and_context;
use crate::session::turn_context::TurnContext;
use crate::tools::context::ExecCommandToolOutput;
use crate::unified_exec::WriteStdinRequest;
use codex_utils_output_truncation::TruncationPolicy;
use core_test_support::skip_if_sandbox;
use pretty_assertions::assert_eq;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::time::Duration;
use tokio::time::Instant;

async fn test_session_and_turn() -> (Arc<Session>, Arc<TurnContext>) {
    let (session, turn) = make_session_and_context().await;
    (Arc::new(session), Arc::new(turn))
}

async fn exec_command(
    session: &Arc<Session>,
    turn: &Arc<TurnContext>,
    cmd: &str,
    yield_time_ms: u64,
    workdir: Option<PathBuf>,
) -> Result<ExecCommandToolOutput, UnifiedExecError> {
    exec_command_with_tty(
        session,
        turn,
        cmd,
        yield_time_ms,
        workdir,
        /*tty*/ true,
    )
    .await
}

async fn exec_command_with_tty(
    session: &Arc<Session>,
    turn: &Arc<TurnContext>,
    cmd: &str,
    yield_time_ms: u64,
    workdir: Option<PathBuf>,
    tty: bool,
) -> Result<ExecCommandToolOutput, UnifiedExecError> {
    let manager = &session.services.unified_exec_manager;
    let process_id = manager.allocate_process_id().await;
    #[allow(deprecated)]
    let cwd = workdir
        .as_ref()
        .map_or_else(|| turn.cwd.clone(), |workdir| turn.cwd.join(workdir));
    let command = vec!["bash".to_string(), "-lc".to_string(), cmd.to_string()];
    let context =
        UnifiedExecContext::new(Arc::clone(session), Arc::clone(turn), "call".to_string());
    manager
        .exec_command(
            ExecCommandRequest {
                command,
                shell_type: crate::shell::ShellType::Bash,
                hook_command: cmd.to_string(),
                process_id,
                yield_time_ms,
                max_output_tokens: None,
                cwd,
                environment: turn
                    .environments
                    .primary_environment()
                    .expect("primary environment"),
                network: turn.network.clone(),
                tty,
            },
            &context,
        )
        .await
}

async fn write_stdin(
    session: &Arc<Session>,
    process_id: i32,
    input: &str,
    yield_time_ms: u64,
) -> Result<ExecCommandToolOutput, UnifiedExecError> {
    session
        .services
        .unified_exec_manager
        .write_stdin(WriteStdinRequest {
            process_id,
            input,
            yield_time_ms,
            max_output_tokens: None,
            truncation_policy: TruncationPolicy::Tokens(10_000),
        })
        .await
}

#[test]
fn push_chunk_preserves_prefix_and_suffix() {
    let mut buffer = HeadTailBuffer::default();
    buffer.push_chunk(vec![b'a'; UNIFIED_EXEC_OUTPUT_MAX_BYTES]);
    buffer.push_chunk(vec![b'b']);
    buffer.push_chunk(vec![b'c']);

    assert_eq!(buffer.retained_bytes(), UNIFIED_EXEC_OUTPUT_MAX_BYTES);
    let snapshot = buffer.snapshot_chunks();

    let first = snapshot.first().expect("expected at least one chunk");
    assert_eq!(first.first(), Some(&b'a'));
    assert!(snapshot.iter().any(|chunk| chunk.as_slice() == b"b"));
    assert_eq!(
        snapshot
            .last()
            .expect("expected at least one chunk")
            .as_slice(),
        b"c"
    );
}

#[test]
fn head_tail_buffer_default_preserves_prefix_and_suffix() {
    let mut buffer = HeadTailBuffer::default();
    buffer.push_chunk(vec![b'a'; UNIFIED_EXEC_OUTPUT_MAX_BYTES]);
    buffer.push_chunk(b"bc".to_vec());

    let rendered = buffer.to_bytes();
    assert_eq!(rendered.first(), Some(&b'a'));
    assert!(rendered.ends_with(b"bc"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unified_exec_persists_across_requests() -> anyhow::Result<()> {
    skip_if_sandbox!(Ok(()));

    let (session, turn) = test_session_and_turn().await;

    let open_shell = exec_command(
        &session, &turn, "bash -i", /*yield_time_ms*/ 2_500, /*workdir*/ None,
    )
    .await?;
    let process_id = open_shell.process_id.expect("expected process_id");

    write_stdin(
        &session,
        process_id,
        "export CODEX_INTERACTIVE_SHELL_VAR=codex\n",
        /*yield_time_ms*/ 2_500,
    )
    .await?;

    let out_2 = write_stdin(
        &session,
        process_id,
        "echo $CODEX_INTERACTIVE_SHELL_VAR\n",
        /*yield_time_ms*/ 2_500,
    )
    .await?;
    assert!(
        out_2
            .truncated_output(DEFAULT_MAX_OUTPUT_TOKENS)
            .contains("codex"),
        "expected environment variable output"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn multi_unified_exec_sessions() -> anyhow::Result<()> {
    skip_if_sandbox!(Ok(()));

    let (session, turn) = test_session_and_turn().await;

    let shell_a = exec_command(
        &session, &turn, "bash -i", /*yield_time_ms*/ 2_500, /*workdir*/ None,
    )
    .await?;
    let session_a = shell_a.process_id.expect("expected process id");

    write_stdin(
        &session,
        session_a,
        "export CODEX_INTERACTIVE_SHELL_VAR=codex\n",
        /*yield_time_ms*/ 2_500,
    )
    .await?;

    let out_2 = exec_command(
        &session,
        &turn,
        "echo $CODEX_INTERACTIVE_SHELL_VAR",
        /*yield_time_ms*/ 2_500,
        /*workdir*/ None,
    )
    .await?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        out_2.process_id.is_none(),
        "short command should not report a process id if it exits quickly"
    );
    assert!(
        !out_2
            .truncated_output(DEFAULT_MAX_OUTPUT_TOKENS)
            .contains("codex"),
        "short command should run in a fresh shell"
    );

    let out_3 = write_stdin(
        &session,
        shell_a.process_id.expect("expected process id"),
        "echo $CODEX_INTERACTIVE_SHELL_VAR\n",
        /*yield_time_ms*/ 2_500,
    )
    .await?;
    assert!(
        out_3
            .truncated_output(DEFAULT_MAX_OUTPUT_TOKENS)
            .contains("codex"),
        "session should preserve state"
    );

    Ok(())
}

#[tokio::test]
async fn unified_exec_timeouts() -> anyhow::Result<()> {
    skip_if_sandbox!(Ok(()));

    const TEST_VAR_VALUE: &str = "unified_exec_var_123";

    let (session, turn) = test_session_and_turn().await;

    let open_shell = exec_command(
        &session, &turn, "bash -i", /*yield_time_ms*/ 2_500, /*workdir*/ None,
    )
    .await?;
    let process_id = open_shell.process_id.expect("expected process id");

    write_stdin(
        &session,
        process_id,
        format!("export CODEX_INTERACTIVE_SHELL_VAR={TEST_VAR_VALUE}\n").as_str(),
        /*yield_time_ms*/ 2_500,
    )
    .await?;

    let out_2 = write_stdin(
        &session,
        process_id,
        "sleep 5 && echo $CODEX_INTERACTIVE_SHELL_VAR\n",
        /*yield_time_ms*/ 10,
    )
    .await?;
    assert!(
        !out_2
            .truncated_output(DEFAULT_MAX_OUTPUT_TOKENS)
            .contains(TEST_VAR_VALUE),
        "timeout too short should yield incomplete output"
    );

    tokio::time::sleep(Duration::from_secs(7)).await;

    let out_3 = write_stdin(&session, process_id, "", /*yield_time_ms*/ 100).await?;

    assert!(
        out_3
            .truncated_output(DEFAULT_MAX_OUTPUT_TOKENS)
            .contains(TEST_VAR_VALUE),
        "subsequent poll should retrieve output"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unified_exec_pause_blocks_yield_timeout() -> anyhow::Result<()> {
    skip_if_sandbox!(Ok(()));

    let (session, turn) = test_session_and_turn().await;
    session.set_out_of_band_elicitation_pause_state(/*paused*/ true);

    let paused_session = Arc::clone(&session);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        paused_session.set_out_of_band_elicitation_pause_state(/*paused*/ false);
    });

    let started = tokio::time::Instant::now();
    let response = exec_command(
        &session,
        &turn,
        "sleep 1 && echo unified-exec-done",
        /*yield_time_ms*/ 250,
        /*workdir*/ None,
    )
    .await?;

    assert!(
        started.elapsed() >= Duration::from_secs(2),
        "pause should block the unified exec yield timeout"
    );
    assert!(
        response
            .truncated_output(DEFAULT_MAX_OUTPUT_TOKENS)
            .contains("unified-exec-done"),
        "exec_command should wait for output after the pause lifts"
    );
    assert!(
        response.process_id.is_none(),
        "completed command should not leave a background process"
    );

    Ok(())
}

#[tokio::test]
#[ignore] // Ignored while we have a better way to test this.
async fn requests_with_large_timeout_are_capped() -> anyhow::Result<()> {
    let (session, turn) = test_session_and_turn().await;

    let result = exec_command(
        &session,
        &turn,
        "echo codex",
        /*yield_time_ms*/ 120_000,
        /*workdir*/ None,
    )
    .await?;

    assert!(result.process_id.is_some());
    assert!(
        result
            .truncated_output(DEFAULT_MAX_OUTPUT_TOKENS)
            .contains("codex")
    );

    Ok(())
}

#[tokio::test]
#[ignore] // Ignored while we have a better way to test this.
async fn completed_commands_do_not_persist_sessions() -> anyhow::Result<()> {
    let (session, turn) = test_session_and_turn().await;
    let result = exec_command(
        &session,
        &turn,
        "echo codex",
        /*yield_time_ms*/ 2_500,
        /*workdir*/ None,
    )
    .await?;

    assert!(
        result.process_id.is_some(),
        "completed command should report a process id"
    );
    assert!(
        result
            .truncated_output(DEFAULT_MAX_OUTPUT_TOKENS)
            .contains("codex")
    );

    assert!(
        session
            .services
            .unified_exec_manager
            .process_store
            .lock()
            .await
            .processes
            .is_empty()
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reusing_completed_process_returns_unknown_process() -> anyhow::Result<()> {
    skip_if_sandbox!(Ok(()));

    let (session, turn) = test_session_and_turn().await;

    let open_shell = exec_command(
        &session, &turn, "bash -i", /*yield_time_ms*/ 2_500, /*workdir*/ None,
    )
    .await?;
    let process_id = open_shell.process_id.expect("expected process id");

    write_stdin(&session, process_id, "exit\n", /*yield_time_ms*/ 2_500).await?;

    tokio::time::sleep(Duration::from_millis(200)).await;

    let err = write_stdin(&session, process_id, "", /*yield_time_ms*/ 100)
        .await
        .expect_err("expected unknown process error");

    match err {
        UnifiedExecError::UnknownProcessId { process_id: err_id } => {
            assert_eq!(err_id, process_id, "process id should match request");
        }
        other => panic!("expected UnknownProcessId, got {other:?}"),
    }

    assert!(
        session
            .services
            .unified_exec_manager
            .process_store
            .lock()
            .await
            .processes
            .is_empty()
    );

    Ok(())
}
