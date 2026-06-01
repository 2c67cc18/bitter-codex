# App-Server Archive Resume Guard

## Upstream Reference

- `3e7baa00e` - add thread archive CLI commands

Classification: `manual-port`.

Reason: the upstream commit mostly adds CLI/TUI archive commands, but it also
fixes retained app-server behavior: archived sessions addressed by id should
not resume or fork as active sessions.

## Goal

Port the surviving app-server guard for archived sessions.

## Behavior

- when reading a stored thread for resume/fork by id, reject archived sessions
  with an invalid-request error
- include a user-actionable `codex unarchive <thread id>` hint in the error
- keep existing app-server archive/unarchive RPC behavior unchanged

## Adapt Out

Do not port:

- `codex archive` / `codex unarchive` CLI commands
- TUI archive command plumbing
- upstream remote endpoint helper reshaping that exists only to support those
  CLI commands

## Tests

Port or adapt upstream coverage only where it maps to surviving behavior:

- `3e7baa00e`: `thread/resume` by archived id fails
- `3e7baa00e`: the error message includes the matching unarchive command

## Validation

Run after implementation:

- `cargo +stable fmt`
- `cargo +stable test -p codex-app-server thread_resume`
