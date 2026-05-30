# Restore Remote Compaction V2

## Upstream References

- `d927f6120` - add remote compaction v2 Responses client path
- `dfa1e864a` - send `response.processed` after remote compaction v2
- `0322ac3df` - use `compaction_trigger` item for remote compaction v2
- `94442b7f9` - retain remote compaction truncation parity in v2
- `dac98cb63` - retry remote compaction v2 requests
- `04a8580f3` - centralize Responses retry policy
- `9271e84b7` - add manual and `remote_v2` compaction metric tags
- `bee78806a` - add compaction metadata to turn headers

Classification: `manual-port`.

Reason: this fork previously had `compact_remote_v2.rs`, then removed it in
local trimming commits. The upstream implementation now also references removed
hooks, analytics, rollout-trace, personality, and app/TUI surfaces.

## Goal

Restore remote compaction v2 as one coherent local commit.

Remote compaction v2 should use the normal streamed Responses client instead of
the unary `/responses/compact` endpoint. It should install the returned
compaction output into history and keep the legacy remote compact endpoint as
fallback.

## Selection

Use v2 when the provider supports remote compaction.

Otherwise:

- non-remote provider: use local compaction

Do not restore a `remote_compaction_v2` feature flag. This fork keeps only a
small runtime feature set, and v2 should be the default remote compaction path.

Keep the existing unary `/responses/compact` implementation available only as a
local fallback during the port if needed; it should not require a user-visible
feature switch.

Apply the selection to:

- automatic pre-turn compaction
- model-downshift pre-turn compaction
- mid-turn follow-up compaction
- manual compact task

## Restore Mechanically

Restore as much as possible from upstream `rust-v0.135.0`:

- `codex-rs/core/src/compact_remote_v2.rs`
- `mod compact_remote_v2`
- session turn wiring
- manual compact task wiring
- retry helper from `responses_retry.rs`
- compaction metric helper and `remote_v2` tag
- compaction turn metadata header support

Keep existing local code when it already has the needed protocol support:

- `ResponseItem::ContextCompaction`
- `ResponseItem::CompactionTrigger`
- `ContextCompactionItem`
- turn metadata header plumbing
- `response.processed` websocket request

Current code does not have `core/src/compact_remote_v2.rs` or
`core/src/responses_retry.rs`; those are restored/adapted in this bundle.

## Adapt Out

Do not restore removed surfaces just for this feature:

- hooks
- analytics crate/events
- rollout trace
- personality
- collaboration mode
- app/TUI tests
- MCP/tool-search-only behavior

Where upstream v2 calls those surfaces, remove or replace with local equivalents.

## Behavior

The v2 request should:

- clone current history
- trim generated function-call history to fit the context window
- append `ResponseItem::CompactionTrigger`
- stream a normal Responses request
- require exactly one `ResponseItem::Compaction` output
- allow other output items
- retain only installed-history input shape before adding the compaction output
- truncate retained messages to the upstream budget
- process compacted history through existing local `process_compacted_history`
- emit context-compaction start/completed items
- recompute token usage
- send `response.processed` after successful v2 compaction when
  `responses_websocket_response_processed` is enabled

The v2 path should reuse the current turn `ModelClientSession` for automatic
compaction. Manual compaction may create its own session.

## Retry

Use a shared retry helper for Responses stream errors:

- same websocket-to-HTTPS fallback behavior as sampling
- max v2 compact retries capped at `2`
- retry notification behavior matching sampling
- compact-specific failure logging on final failure

## Metrics

Emit `codex.task.compact` with:

- `type=remote_v2`
- `manual=true|false`

Keep existing `local` and `remote` tags for other paths.

## Tests

Port or adapt upstream coverage only where it maps to surviving behavior:

- `d927f6120` / `0322ac3df`: remote-capable compaction uses the v2 streamed
  Responses path with `compaction_trigger` input and exactly one `compaction`
  output
- `d927f6120`: manual and automatic compaction route through v2 when the
  provider supports remote compaction
- `dfa1e864a`: `response.processed` is sent after successful v2 compaction
  when the existing feature flag is enabled
- `94442b7f9`: retained history shape and truncation match the legacy remote
  compaction contract
- `dac98cb63` / `04a8580f3`: retry and websocket-to-HTTPS fallback behavior
  matches normal Responses streaming with the compact retry budget
- `9271e84b7`: compaction metric tags include `manual` and `remote_v2`
- `bee78806a`: compaction requests include the upstream metadata fields that
  survive in this fork

## Related Flag Handling

Keep `responses_websocket_response_processed` default false for now. The v2
compaction path should follow the existing normal-turn behavior and send
`response.processed` only when the flag is enabled.

Do not port tests for removed hooks, analytics, rollout trace, TUI, or MCP.

## Validation

Run after implementation:

- `cargo +stable fmt`
- `cargo +stable test -p codex-core compact_remote`
- `cargo +stable test -p codex-core`
- `cargo +stable test --workspace`
