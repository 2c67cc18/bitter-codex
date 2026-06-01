# App-Server Thread History / Client IDs

## Upstream References

- `2a1158b8e` - include turns page on thread resume
- `e92c952b2` - add user input client IDs

Classification: `manual-port`.

Reason: app-server protocol v2, thread resume, and thread-history conversion
survive locally, but upstream changes also include TUI, analytics,
app-server-test-client, and generated schema churn that should be adapted to
the local API surface.

## Goal

Port the surviving app-server resume/history behavior:

- let `thread/resume` optionally return an initial turns page
- preserve client-provided IDs on user input items and reconstructed thread
  history

## Behavior

Thread resume:

- add the upstream `initialTurnsPage` request shape if the local v2 API does
  not already expose it
- return a page of turns from `thread/resume` without changing the existing
  cold-resume and cached-resume behavior
- keep redaction behavior for the returned page aligned with `thread/read`

Client IDs:

- accept client IDs on v2 turn start/steer user input where the local protocol
  supports those input variants
- preserve those IDs through `ThreadItem`/thread-history conversions
- include IDs in app-server-emitted started/completed items where those items
  survive locally

## Adapt Out

Do not restore or copy upstream-only surfaces:

- analytics events/tests
- TUI session handling
- app-server-test-client
- removed request groups or generated fixtures unrelated to this API

## Tests

Port or adapt upstream coverage only where it maps to surviving behavior:

- `2a1158b8e`: v2 protocol serialization for resume initial turns page
- `2a1158b8e`: `thread/resume` returns the requested initial turns page
- `2a1158b8e`: resume page redaction matches `thread/read`
- `e92c952b2`: user input client IDs round-trip through v2 turn params
- `e92c952b2`: thread-history reconstruction preserves client IDs

## Validation

Run after implementation:

- `cargo +stable fmt`
- `cargo +stable test -p codex-app-server-protocol`
- `cargo +stable test -p codex-app-server thread_resume`
- `cargo +stable test -p codex-app-server thread_read`
