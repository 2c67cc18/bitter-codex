# Web Tool Runtime

## Upstream References

- `96f1347fa` - show activity for standalone web search calls
- `1f93706e9` - require model for standalone web search
- `1c55bb270` - improve built-in tool schema docs

Classification: `manual-port`.

Reason: local standalone web search and some built-in tool descriptions
survive. Upstream activity emission is implemented through the removed
`ext/web-search` extension, so the behavior must be adapted to the local
core-level `WebSearchHandler`.

## Goal

Port the web runtime behavior that still applies locally:

- keep standalone web-search requests tied to the active model
- expose standalone web-search activity in surviving core events
- improve descriptions only for retained built-in tools

## Behavior

Standalone web search:

- include the active model in standalone search requests where the local
  `codex-api` search types support it
- preserve direct-only standalone web-search settings from the previous local
  web-tool port
- emit started/completed `TurnItem::WebSearch` activity from the local
  `WebSearchHandler` using `Session::emit_turn_item_started` and
  `Session::emit_turn_item_completed`
- derive activity detail for search queries, image queries, literal-URL
  `open`, and `find` commands using the upstream intent

Tool descriptions:

- update retained shell, view-image, and web-search tool descriptions when the
  upstream wording applies
- do not add descriptions for removed code-mode, MCP resource, multi-agent,
  goal, or test-sync tools

## Tests

Port or adapt upstream coverage only where it maps to surviving behavior:

- `96f1347fa`: standalone web-search emits readable activity details
- `96f1347fa`: completed standalone web-search activity is persisted and
  reconstructed through thread history where local app-server coverage exists
- `1f93706e9`: standalone web-search request includes the active model
- `1c55bb270`: retained built-in tool descriptions match the upstream intent

## Validation

Run after implementation:

- `cargo +stable fmt`
- `cargo +stable test -p codex-api search`
- `cargo +stable test -p codex-core web_search`
- `cargo +stable test -p codex-core hosted_spec`
