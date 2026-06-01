# Dynamic Tool Storage Cleanup

## Upstream Reference

- `304d15cab` - remove redundant SQLite dynamic tool storage

Classification: `manual-port`.

Reason: dynamic tools, rollout `SessionMeta`, SQLite state, and thread-store
metadata still exist locally. The upstream patch removes a duplicate
persistence path and should be adapted to the local state/thread-store layout.

## Goal

Stop persisting dynamic tools through SQLite/thread metadata and rely on rollout
session metadata for resume/fork reconstruction.

## Behavior

- restore missing thread-start dynamic tools from rollout `SessionMeta`
- remove SQLite reads, writes, extraction helpers, and metadata patches for
  dynamic tools
- leave the existing `thread_dynamic_tools` table in place for mixed-version
  compatibility
- keep dynamic tool definitions available after resume and fork

## Tests

Port or adapt upstream coverage only where it maps to surviving behavior:

- `304d15cab`: SQLite-enabled resume restores dynamic tools from rollout
  metadata
- `304d15cab`: thread metadata sync no longer writes dynamic-tool patches
- `304d15cab`: state runtime no longer reads or writes `thread_dynamic_tools`

## Validation

Run after implementation:

- `cargo +stable fmt`
- `cargo +stable test -p codex-state`
- `cargo +stable test -p codex-thread-store`
- `cargo +stable test -p codex-core sqlite_state`
- `cargo +stable test --workspace`
