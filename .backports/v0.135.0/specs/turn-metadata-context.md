# Turn Metadata / Context Bundle

## Upstream References

- `b637fd26a` - make active turn task singular
- `1911021c0` - add `forked_from_thread_id` turn metadata
- `768848ab6` - experimental turn additional context

Classification: `manual-port`.

Reason: these commits overlap in session turn state, task routing, and metadata
plumbing, but upstream patches also touch removed surfaces and broad generated
test fixtures.

## Goal

Port the surviving turn-shape and metadata behavior as one local change.

The local implementation should preserve:

- one active turn task per session
- fork lineage in turn metadata
- app-server-provided additional context on turns

## Behavior

Active turn task:

- replace multi-active-turn state with one active turn task
- keep pending-input and cancellation behavior unchanged
- preserve network approval routing to the active turn

Fork metadata:

- carry `forked_from_thread_id` through thread resume/fork flows
- include it in turn metadata headers sent to Responses
- expose it through surviving app-server v2 metadata paths

Additional context:

- accept additional context on app-server v2 turn start/steer requests
- store it with session/turn state as needed
- render it into model-visible context using existing context fragment patterns
- avoid restoring goal, guardian, plugin, memories, MCP, or TUI-only plumbing

## Tests

Port or adapt upstream coverage only where it maps to surviving behavior:

- `b637fd26a`: existing active-turn lifecycle tests for steer, abort,
  pending input, goal reservation, and network approval still pass with the
  singular task slot
- `1911021c0`: fork lineage appears in turn metadata and app-server Responses
  API request metadata
- `768848ab6`: app-server v2 `turn/start` and non-empty `turn/steer` accept
  `additionalContext`
- `768848ab6`: additional context is injected as hidden contextual input with
  the upstream role, dedupe/reset, deletion/re-add, empty-steer rejection, and
  truncation behavior

Do not copy broad fixture updates or tests for removed surfaces.

## Validation

Run after implementation:

- `cargo +stable fmt`
- `cargo +stable test -p codex-app-server-protocol`
- `cargo +stable test -p codex-app-server`
- `cargo +stable test -p codex-core`
