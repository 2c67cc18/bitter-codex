# App-Server V2 Test Surface

## Upstream References

- `8287f7a16` - local fork removed the upstream app-server integration tests
- `5cd9b8086a0b1d46121019506634abb6910a7617` - resume cached-thread override coverage
- `61cbf3574eca870df6fa7f49648ec7e001901b5a` - fork rollout truncation coverage

Classification: `local-restoration`.

Reason: app-server protocol v2 is still a retained surface, so backports that
change v2 behavior need an adapted v2 test surface.

## Goal

Restore enough app-server v2 integration coverage to support current and future
manual ports without reintroducing tests for removed product surfaces.

## Scope

Restore or adapt:

- aggregate app-server integration test entrypoint
- shared test support needed by retained v2 APIs
- `thread_resume` coverage for cached idle resume overrides
- `thread_unsubscribe` coverage that exercises cached resume state

Do not restore v2 tests for removed surfaces unless a later backport restores
the corresponding runtime/API behavior.

## Retained V2 Test Policy

Keep v2 tests when the tested API and runtime path both still exist locally.

Prioritize:

- thread lifecycle: start, resume, read, list, status, unsubscribe, archive
- turn lifecycle: start, steer, interrupt
- config/model/account APIs that still compile against local app-server
- dynamic tools and tool I/O behavior that remains in protocol/core

Defer or skip tests for surfaces removed from this fork, such as plugins,
marketplace, realtime, analytics, collaboration, MCP, memories, remote thread
store, sandbox setup, hooks, skills, and web search until those surfaces are
explicitly restored.

## Resume Bundle Dependency

Before merging the resume/rollout-history lane, restore enough v2 coverage to
adapt upstream `5cd9b8086a0b1d46121019506634abb6910a7617` tests:

- idle cached loaded thread with cwd/config override unloads and cold-resumes
- subscribed or active loaded thread still rejoins existing state
- `thread_unsubscribe` cached-status behavior still covers resume with cwd

If `61cbf3574eca870df6fa7f49648ec7e001901b5a` has no surviving last-N fork
surface locally, do not add dead helper tests. Record that absence in the
resume bundle commit body.

## Validation

Run after implementation:

- `cargo +stable fmt`
- `cargo-modal --dirty --cache home test -p codex-app-server`
- focused test filters for restored v2 modules when available

