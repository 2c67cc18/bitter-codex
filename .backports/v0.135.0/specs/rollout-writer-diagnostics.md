# Rollout Writer Diagnostics

## Upstream Reference

- `e8651516f` - Log rollout writer OS errors

Classification: `manual-port`.

Reason: the patch does not apply cleanly to the local rollout recorder, but the
surviving behavior is a small diagnostic-only edit.

## Goal

Include `ErrorKind` and `raw_os_error()` in rollout writer failure logs.

## Behavior

- preserve the append-only rollout write path
- add OS error details to terminal writer-task failure logs
- add OS error details to buffered write retry logs

## Tests

Port upstream coverage only if local rollout tests already assert these log
paths. Do not add unrelated logging tests.

## Validation

Run after implementation:

- `cargo +stable fmt`
- `cargo +stable test -p codex-rollout`
