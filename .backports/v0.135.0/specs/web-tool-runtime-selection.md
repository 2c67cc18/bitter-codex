# Runtime-Selected Web Tool

## Upstream Reference

- Commit: `a22706dfa` (`standalone websearch extension`)
- Follow-up: `66ff8b0f5` (`make direct only allowed caller for standalone websearch`)
- Follow-up: `9fe55d68e` (`fix: dont compact standalone websearch schema`)
- Classification: `manual-port`
- Reason: upstream implements `web.run` through the extension registry, which
  is not a carried surface in this fork.

## Goal

Add a local Codex-executed web tool named `web`, based on the upstream
standalone search behavior, while keeping hosted `web_search` as a separate
runtime-selected tool.

For each turn, app-server selects one web surface:

- `web_search`: hosted Responses API tool
- `web`: local Codex-executed JSON tool
- none

`web_search` and `web` must never be exposed together.

## Non-Goals

- Do not port the upstream extension crate.
- Do not port extension registry wiring.
- Do not add TOML/config selection for the tool implementation.
- Do not rename hosted `web_search`.

## Runtime Selection

App-server owns the implementation choice and passes it into turn/tool planning.

Core/tool planning should treat the selected web surface as runtime input:

- hosted: include `ToolSpec::WebSearch`
- local: include the `web` function tool and executor
- none: include neither

Existing web search mode/config may still control behavior such as live/cached,
location, domains, and context size. It must not choose hosted vs local.

The existing feature flags `web_search_request` and `web_search_cached` should
be removed or replaced as part of this work. App-server runtime selection should
decide whether the turn gets hosted `web_search`, local `web`, or neither.

## Local `web` Tool

The local tool should accept one JSON object matching the upstream
`SearchCommands` shape:

- `search_query`
- `image_query`
- `open`
- `click`
- `find`
- `screenshot`
- `finance`
- `weather`
- `sports`
- `time`
- `response_length`

The tool should call the standalone search endpoint through
`codex-api::SearchClient` and return the encrypted search output to Responses.
Search settings should set `allowed_callers` to `["direct"]`; upstream does not
support this standalone path in code mode.

## Portable Upstream Pieces

Consider manually porting:

- `SearchCommands` schema support where local types are incomplete
- search command field docs
- response-history tail selection
- assistant-output truncation helper
- standalone search request construction

Avoid copying upstream extension-specific types or tests.

Current code already has `codex-api::SearchClient`, `SearchRequest`, and
`SearchCommands`; reuse those instead of porting duplicate API types.

## Tests

Port or adapt upstream coverage only where it maps to surviving behavior:

- `a22706dfa`: hosted and standalone web search remain mutually exclusive
- `a22706dfa`: local `web` serializes/deserializes the upstream command JSON
- `a22706dfa`: local `web` sends the expected `SearchRequest`
- `a22706dfa`: response-history helper keeps the intended context tail
- `66ff8b0f5`: local `web` sets `allowed_callers` to `["direct"]`
- `9fe55d68e`: standalone web schema keeps the field guidance that upstream
  preserves by bypassing schema compaction

Test app-server runtime selection only as the local adaptation of upstream's
standalone-vs-hosted exclusivity. Do not copy tests for extension registration.
