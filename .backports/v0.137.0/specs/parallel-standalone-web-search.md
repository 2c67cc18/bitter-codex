# Parallel Standalone Web Search

## Upstream Reference

- `b3c4157034` - enable parallel standalone web search calls

Classification: `manual-port`.

Reason: upstream implements this in the extension-backed `web.run` tool, while
this fork moved the behavior into the native/dynamic tool runtime and retains
standalone web search as a local core tool handler. The behavior maps to the
surviving tool runtime, but the upstream patch shape does not apply directly.

## Goal

Allow independent standalone web-search calls to run concurrently when model
parallel tool calls are enabled.

## Behavior

- make the retained standalone web-search handler advertise parallel-call
  support
- leave hosted web-search tool selection and request payloads unchanged
- do not restore the upstream web-search extension surface or extension
  registration tests

## Tests

Port or adapt upstream coverage only where it maps to surviving behavior:

- `b3c4157034`: standalone web-search tool metadata reports parallel-call
  support
- `b3c4157034`: existing web-search registration/plan tests still select the
  local standalone handler when configured

## Validation

Run after implementation:

- `cargo +stable fmt`
- `cargo +stable test -p codex-core web_search`
- because this touches `codex-core`, run `cargo +stable test --workspace`
  after focused tests pass
