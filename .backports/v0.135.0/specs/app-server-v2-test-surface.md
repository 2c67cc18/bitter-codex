# App-Server V2 Test Surface

Classification: `local-restoration`.

Reason: app-server protocol v2 is still a retained surface, so its integration
test harness should exist independently of any one backport.

## Goal

Restore the app-server integration test harness and retained v2 coverage.

## Scope

Restore mechanically first:

- aggregate app-server integration test entrypoint
- shared app-server test support crate
- v2 suite module tree

Then remove or adapt tests for runtime/API surfaces that no longer exist in
this fork.

## Retained Surface

Keep v2 tests when the tested API and runtime path both still exist locally.

Retained request groups include:

- thread lifecycle: start, resume, read, list, status, unsubscribe, archive
- turn lifecycle: start, steer, interrupt
- config, model, and account APIs that still compile against local app-server
- dynamic tools and tool I/O behavior that remains in protocol/core
- server notifications for retained thread, turn, item, account, model, and
  config warning events

Remove or defer tests for surfaces removed from this fork, such as plugins,
marketplace, realtime, analytics, collaboration, MCP, memories, remote thread
store, sandbox setup, hooks, skills, and web search.

## Implementation Notes

- prefer restoring upstream files as-is before pruning
- keep test helpers broad enough for later retained v2 test modules
- avoid reintroducing production dependencies solely for removed tests
- keep removal/adaptation decisions visible in the commit body

## Validation

Run after implementation:

- `cargo +stable fmt`
- `cargo-modal --dirty --cache home test -p codex-app-server`
- focused filters for restored v2 modules when useful
