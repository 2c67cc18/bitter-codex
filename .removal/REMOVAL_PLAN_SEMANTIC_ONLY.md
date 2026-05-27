# Codex Rust semantic-only removal plan

This file tracks follow-up edits that require code understanding and should not be
done as blind deletion or line-range cleanup.

## Cargo/default dependency architecture

- Edit surviving crate manifests so the default graph contains no removed optional
  stacks: MCP, code-mode/V8, tool-search/deferred discovery, schema/codegen, TUI,
  sandbox/approval/execpolicy, and analytics.
- Remove feature gates for removed capabilities rather than preserving them as
  optional code paths.
- Keep only Cargo as the supported build/test interface.

## Analytics removal follow-through

- Remove `codex-analytics` use sites from app-server, core session/state, thread
  manager, CLI/app-server flags, config, tests, and any helper modules.
- Remove `analytics_enabled`, `analytics.default`, and
  `--analytics-default-enabled` configuration/CLI surfaces.
- Remove tracking calls for initialize/request/response/notification/turn/token
  usage/compaction/hook/plugin/app/subagent/permission events.
- Remove analytics mock servers and analytics assertions from retained tests.
- Decide whether any local-only logging or counters should replace telemetry; do
  not send usage events to a backend.

## Extension API removal follow-through

- The `codex-rs/ext/extension-api` crate and manifest edges were removed
  mechanically; remove all remaining `codex_extension_api` imports and concepts
  from core, core-api, app-server, tests, and helpers.
- Remove extension registries/builders, extension data stores, prompt/config/tool
  contributors, response-item injection, and thread/turn/tool lifecycle
  contributor dispatch.
- Replace empty-extension-registry plumbing with ordinary retained constructors
  and state fields that do not mention extensions.
- Delete or rewrite tests that only validate extension contributor behavior.
- Remove orphaned extension-era tool invocation API from `codex-rs/tools`:
  delete `src/tool_call.rs`, remove the `tool_call` module and
  `ConversationHistory`/`ToolCall` re-exports from `src/lib.rs`, and update the
  crate README so it no longer claims retained shared `ToolCall` executable-tool
  contracts.

## Hooks removal follow-through

- The `codex-rs/hooks` crate, generated hook schemas, and manifest edges were
  removed mechanically; remove all remaining hook config, protocol, app-server,
  exec, telemetry, and test plumbing.
- Remove hook config parsing/requirements/trust handling from `codex-config` and
  config loader/managed config paths.
- Remove app-server hook listing/config mapping and hook request/notification
  protocol types/events.
- Remove core hook lifecycle emission around session start/stop, user prompt
  submit, tool start/finish, compact, and subagent events.
- Remove hook-related CLI flags such as hook trust bypass and any hook output
  spill/log handling.

## Auth simplification

- Keep stored API key auth, provider `env_key` auth, ChatGPT browser OAuth,
  ChatGPT device-code OAuth, and externally supplied ChatGPT tokens through
  app-server.
- Remove agent identity auth, `CODEX_ACCESS_TOKEN`, generic command-backed bearer
  auth, provider command auth, AWS/Bedrock auth, and related provider fields.
- Retain `ChatgptAuthTokensRefresh` for externally supplied ChatGPT tokens.
- `CODEX_ACCESS_TOKEN` implicit auth loading and CLI `codex login
  --with-access-token` were removed from main. Stored agent-identity auth
  records and provider signing support still remain and belong to the remaining
  auth-provider cleanup.

## Model provider/catalog simplification

- Remove AWS/Bedrock provider structs, constants, factories, and SigV4 signing.
- Remove provider command-backed bearer auth.
- Decide whether local unauthenticated providers such as Ollama/LM Studio remain.
- Keep local static models only for `gpt-5.5` and `gpt-5.4-mini`; accept
  `gpt-5.3-codex-spark` only from the server/model catalog path.
- Remove `apply_patch_tool_type` and code that reads it.
- Keep hosted web/image capability metadata, but expose tools only when session
  settings enable them.

## App-server protocol trim

- Remove all experimental API runtime gating and capability exposure.
- Remove schema/codegen/TS/JSON-schema export logic that remains in macros or
  retained types.
- Remove v1 approval/sandbox types unless minimum initialize compatibility needs
  them.
- Make retained methods ordinary stable APIs: `thread/settings/update`,
  `thread/backgroundTerminals/clean`, `thread/search`, and `thread/turns/list`.
- Add and wire `HostedToolsConfig { web_search: bool, image_generation: bool }`
  through thread start/resume/fork/settings update, defaulting both to `false`.
- Remove structural fields for environments, permissions, approval policy,
  sandbox, network policy/proxy, apps, memory, realtime, personality, and
  collaboration mode.

## App-server implementation trim

- Refactor request routing/processors around initialize, account/auth, model
  list/capabilities, thread lifecycle/read/list/archive/unarchive/name/metadata,
  settings update, background terminal cleanup, thread search, turns list,
  turn start/steer/interrupt, dynamic tool calls, ChatGPT token refresh, and
  trimmed config read/write.
- Update message processing, outgoing messages, event handling, thread history,
  and item/event mapping to the smaller protocol.
- Remove standalone command/process APIs, filesystem APIs, MCP, plugins,
  marketplace, apps, feedback, remote control, review/guardian/goals/hooks/skills
  surfaces where no longer retained.

## Unix-socket app-server transport

- Keep Unix socket transport and WebSocket framing over Unix streams.
- Remove stdio transport, TCP WebSocket listening/auth if no TCP endpoint remains,
  remote-control enrollment/segments/client tracking, and capability-token/signed
  bearer transport auth.
- Make `codex-uds` Unix-only and remove Windows `uds_windows` support.

## App-server daemon

- Rewrite daemon behavior to local lifecycle only: start, stop, restart, status,
  and version.
- Remove durable install/bootstrap, managed install/update loops, standalone
  updater, and remote-control client behavior.

## CLI rewrite

- Rewrite `cli/src/main.rs` to be subcommand-first; bare `codex` must print help
  or error and must not start a TUI.
- Retain only `codex exec`, `codex exec resume`, `codex login`, `codex logout`,
  Unix-socket app-server, app-server daemon lifecycle, and app-server proxy.
- Remove interactive TUI/root prompt, interactive resume/fork, review, plugin,
  marketplace, remote-control, desktop app launcher, update, completion unless
  explicitly retained, doctor unless rewritten narrowly, sandbox/debug sandbox,
  execpolicy, apply, cloud, responses-api-proxy, stdio-to-uds, exec-server,
  feature CLI, and access-token/agent-identity login.

## `codex exec` semantic trim

- Keep non-interactive execution and useful options: model, cwd, image, json,
  output-last-message, ephemeral, strict-config, ignore-user-config, and
  output-schema if structured final output remains.
- Remove review mode, approval handling, apply-patch approval handling, command
  approval handling, permission requests, sandbox summaries, exec-server runtime
  paths, environment manager, cloud requirements, feedback, execpolicy, and
  apply-patch dev/runtime paths.

## Core session/runtime simplification

- Refactor core around client/model transport, session/thread/turn loop, tool
  registry/router, dynamic tools, local `exec_command`/`write_stdin`, local
  `view_image`, hosted web/image session gates, and rollout/thread store.
- Remove turn/session state for environments, approval policy, permission
  profiles, sandbox permissions, network proxy/policy, execpolicy, remote
  environment selection, filesystem abstraction, agents/subagents, memory mode,
  realtime state, personality/collaboration mode, turn diff tracker, and plan
  state.
- Remove AGENTS.md discovery/loading/instruction injection and related agent
  instruction merge behavior, or replace it with a deliberately retained local
  project-instructions mechanism if one is still wanted.
- Remove repo/root AGENTS.md documentation and tests after deciding whether any
  non-agent project instruction file remains.

## Home/state compatibility break

- Use a distinct default home/state root from Codex, not `~/.codex` or any
  equivalent Codex-compatible path.
- Do not load existing Codex config, auth, thread store, logs, memories, goals,
  agent jobs, remote-control enrollments, or other databases by default.
- Treat current state schemas as a new product boundary rather than a migrated
  Codex database; remove legacy migration chains once the retained state tables
  are chosen.
- Replace migration tests/fixtures with fresh-database initialization coverage
  for the retained state only.
- If an explicit import path is ever desired, design it separately from normal
  startup so accidental compatibility with Codex user data is impossible.

## Local `exec_command` runtime

- Make `exec_command` the only local mutating model tool.
- Parse args, resolve local cwd/workdir, build local shell argv/env, spawn via
  `codex-utils-pty`, stream output, and keep live process handles for
  `write_stdin`.
- Remove apply-patch interception, legacy shell tool, sandbox retry/escalation,
  approval requests, execpolicy checks, network approval/proxy setup, remote
  exec-server path, `environment_id`, `sandbox_permissions`,
  `additional_permissions`, `justification`, and `prefix_rule`.

## Local `view_image`

- Keep `view_image`, but make it local-only.
- Remove `environment_id` from schema and replace environment filesystem access
  with direct local file reads against cwd/path.
- Keep `codex-utils-image` for image decode/resize/detail handling.

## Hosted web/image tools

- Register hosted `web_search` and `image_generation` only when the current
  app-server thread/session enables them.
- Default both hosted tools to disabled.
- Gate by session setting, model/provider support, and auth/provider permission.

## Dynamic tools

- Keep dynamic tools as a normal protocol feature:
  `thread/start.dynamicTools`, `DynamicToolSpec`, `DynamicToolHandler`,
  `ServerRequest::DynamicToolCall`, `DynamicToolCallParams`, and
  `DynamicToolCallResponse`.
- Remove dynamic-tool experimental gating.

## MCP/code-mode removal

- Remove MCP config parsing, `mcp_servers` fields, startup/status/session
  integration, OAuth/login/resource/tool/elicitation app-server methods, response
  items/events, rollout/thread-history mapping, tool-search/deferred discovery,
  code-mode registration, code-mode item/event handling, V8 service state,
  `Feature::CodeMode`, and code-mode config flags.

## Protocol/model item cleanup

- Remove item/event/protocol types for apply patch, file change, turn diff,
  plan update/delta/items, command/permission approvals, request permissions,
  apps/plugins/marketplace, feedback, remote control, Windows sandbox,
  realtime/voice, memory reset/mode, standalone process/command APIs,
  filesystem APIs, environment APIs, review/guardian/goal/hook/skill APIs if
  removed, subagents/agent jobs, MCP, code-mode, and tool search.
- Keep assistant messages, reasoning summaries, command execution output for
  model tool execution, terminal interaction, view image, hosted web/image
  events/artifact paths, and dynamic tool calls.

## Prompt cleanup

- Clean retained prompts and model instructions in `models-manager/models.json`,
  `models-manager/prompt.md`, protocol default base instructions if retained,
  and image-generation instruction text if retained.
- Review exit XML templates under `core/templates/review/` were removed
  mechanically; remove any remaining review-exit message construction or review
  workflow assumptions from semantic cleanup if review mode is removed.
- Remove mentions of update_plan, apply_patch, approvals, sandboxing, permission
  escalation, network proxy, personality, collaboration modes, subagents, review,
  guardian, memories, interactive TUI, old models, and telemetry/analytics.

## Tests after semantic cleanup

- Rebuild tests around retained surfaces only: API-key auth, provider env-key
  auth, ChatGPT browser/device/external-token auth, Unix-socket app-server,
  Rust client over Unix socket, retained thread lifecycle/settings/search/turns
  methods, turn start/steer/interrupt, dynamic tools, exec_command/write_stdin,
  view_image, hosted web/image gating, rollouts, and thread store.
- Delete or rewrite tests for removed analytics, approvals, sandboxing, MCP,
  plugins, apps, feedback, realtime, memory, TUI, schema/codegen, Bazel/npm/SDK,
  and other removed surfaces.

## Final compile/build check

- After semantic cleanup only, run:

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --no-default-features
cargo test --workspace --no-default-features
```

## Daemex reuse notes

The sibling repo at `/Users/x/Documents/Working/daemex` is a useful
Codex-derived cleanup reference. It is version-skewed from this repo, so prefer
copy/adapt by slice rather than wholesale file replacement unless the diff is
obviously isolated.

- Sandbox / approvals / permissions: very reusable. Daemex has already removed
  Linux sandbox, Windows sandbox, sandboxing crate, permission profiles,
  approval-policy plumbing, network approval, and a lot of related
  protocol/config/test surface. This maps directly onto our `codex exec`, core
  runtime, protocol cleanup, and final dependency pruning work. Treat this as
  the biggest copy/adapt source.
- Execpolicy: very reusable. Daemex commit
  `e67a33f014 Remove execpolicy approval layer` is essentially one of our
  planned semantic slices.
- Home/state compatibility: partially reusable. Daemex has a stronger
  `find_daemex_home()` pattern that rejects upstream `~/.codex`. Adapt that
  idea for the `.bitter-codex` boundary rather than copying names or product
  wording.
- View image/runtime fixes: small and probably copyable, subject to local
  version differences.

## Semantic cleanup DAG and uncertainties

The semantic cleanup should be scheduled as a dependency DAG, not as one
monolithic edit. The graph below is contract-complete but intentionally not
implementation-complete: each slice must report discovered coupling and update
this file or a worker note when it finds new semantic leftovers.

### Root/controller guardrails

- Do not make "compile by re-exporting or reintroducing removed concepts" edits.
  Passing a narrow crate check is not enough if the patch preserves the wrong
  product surface.
- A compile fix is acceptable only when it moves the code toward the retained
  surface described in this plan. If a removed surface is still referenced,
  remove the producer, mapper, event, test, or call site that keeps it alive;
  do not route it through a different crate to satisfy the compiler.
- Do not add local replacement enums/structs for removed crates unless the type
  is explicitly retained in this plan and the replacement is documented as the
  new product boundary.
- Do not re-add or keep app-server notifications for removed surfaces such as
  plan deltas, file-change patch updates, guardian approval reviews, request
  permissions, MCP/config refresh, approval/sandbox prompts, or review/guardian
  workflows merely to make app-server-protocol compile.
- Do not degrade semantic data silently, for example replacing parsed command
  actions with `Unknown`, unless the whole consumer surface is also being
  removed in the same slice.
- Workers must run from fresh worktrees and commit only their own reviewed
  slice. The root may apply small serial integration fixes on main, but it must
  not use broad compile triage to cross feature-slice boundaries.
- When a targeted `cargo-modal` check fails, classify each failure as one of:
  retained surface compile bug, removed surface still referenced, or dependency
  graph fallout. Then schedule the correct semantic slice instead of patching
  the failing crate in isolation.

### Current parallel scheduling contract

The next root should keep parallelism, but only where files and product
surfaces do not overlap:

- Wave A can run in parallel:
  - `git-utils-filesystem-localization`: inspect the current dirty
    `codex-rs/git-utils/src/info.rs` and `lib.rs` edits from the stopped root.
    Decide whether they are a correct local-only replacement for the removed
    `codex_file_system` abstraction. If correct, finish and commit them in a
    fresh worktree or apply a small reviewed serial patch; if not, discard or
    document the better boundary. Do not touch app-server-protocol.
  - `auth-agent-identity-storage`: continue auth simplification by removing
    stored agent-identity auth records and agent-identity helper modules/tests
    after access-token entry points were removed. Do not touch Bedrock/model
    provider code or app-server protocol.
  - `model-provider-bedrock`: remove Bedrock/AWS provider/catalog/signing code
    and its targeted tests. Do not touch auth storage or app-server protocol
    except direct Bedrock references.

- Wave B should wait for Wave A decisions and should not run concurrently with
  each other unless split into strictly disjoint file sets:
  - `execpolicy-approval-runtime`: remove execpolicy amendment/config/runtime
    call sites together with the approval/sandbox/request-permissions concepts
    they depend on, using Daemex as read-only reference.
  - `mcp-code-mode-removal`: remove MCP/code-mode/config refresh protocol,
    app-server processors, core session refresh, and V8/tool-search remnants
    while preserving dynamic tools.

- Wave C must be serial after producers are removed:
  - `app-server-protocol-trim`: remove app-server protocol events/items/mappers
    for surfaces already eliminated upstream. This slice must delete stale
    mappings/notifications rather than reintroducing them. It must not add
    replacements for plan/file-change/guardian/request-permissions unless a
    retained upstream producer still exists and is documented.
  - `app-server-implementation-trim`: update message processing, event mapping,
    thread history, and request processors to the trimmed protocol.
  - `protocol-model-item-cleanup`: prune remaining `codex-protocol` item/event
    variants for removed surfaces after app-server/core producers are gone.

- Wave D is final:
  - `cli-exec-trim`, `prompt-cleanup`, `cargo-feature-dependency-prune`,
    `tests-rebuild`, and `final-checks` through `cargo-modal`.

### Layer 0: completed baseline

- Mechanical baseline removal.
- Analytics semantic removal.
- Hooks semantic removal.
- Extension API semantic removal.
- Narrow daemex sandbox CLI wrapper copy.

### Layer 1: independent small slices

- `tools-orphan-toolcall` (completed): removed the orphaned extension-era
  `codex-rs/tools/src/tool_call.rs` API, its module/re-exports, and README
  claims without touching `codex-rs/core/src/tools/router.rs::ToolCall`.
- `home-state-break` (completed for executable default boundary): made the
  default state/config home `.bitter-codex` and
  prevent implicit loading of upstream `~/.codex` state. This starts with
  `codex-rs/utils/home-dir/src/lib.rs` but may need follow-through in config,
  login/auth storage, state, thread-store, and rollout code.
- `debug-client-removal` (completed): removed the debug client because retained app-server
  workflows no longer need it, especially because it currently models approval
  and sandbox flows.
- `docs-test-stale-removals` (completed for already-removed surfaces): deleted or rewrote docs and tests that only cover
  already-removed features.

### Layer 2: permission/runtime foundation

- `execpolicy-removal`: remove execpolicy approval/amendment/cache logic. Use
  daemex commit `e67a33f014 Remove execpolicy approval layer` as a read-only
  reference. Keep this slice narrow when possible; broader approvals and sandbox
  semantics belong to the next slice.
- `sandbox-approval-permissions-core`: collapse local execution to the retained
  full-access/local model. Remove approval requests, sandbox policy/profile
  management, permission requests, sandbox summaries, network approval/proxy
  plumbing, and related core/protocol/app-server/test surfaces. Use daemex as
  the primary read-only reference.

### Layer 3: major feature removals

- `mcp-code-mode-removal`: remove MCP config/session/app-server/protocol/tool
  plumbing, tool-search/deferred discovery, code-mode registration, V8 service
  state, and code-mode config flags.
- `app-server-protocol-trim`: shrink the app-server protocol around retained
  thread lifecycle/settings/search/turns, dynamic tools, hosted web/image gates,
  auth/account/model surfaces, and Unix-socket transport.
- `cli-exec-trim`: rewrite CLI entrypoints around retained `codex exec`,
  `exec resume`, login/logout, and local app-server/daemon/proxy operations.
- `auth-provider-simplification`: remove agent identity auth, generic command
  auth, provider command auth, AWS/Bedrock auth, and unsupported provider/model
  catalog entries.
- `agents-agentsmd-removal` (completed): removed AGENTS.md discovery/loading/instruction
  injection, repo/root AGENTS.md documentation, and related tests, unless a new
  non-agent project-instruction mechanism is explicitly retained.

### Layer 4: integration cleanup

- `protocol-model-item-cleanup`: remove protocol/model item/event variants for
  deleted features after the upstream feature slices have stopped producing
  them.
- `core-session-runtime-simplification`: simplify session/thread/turn state
  after approvals, sandbox, MCP, code-mode, AGENTS, agents/subagents, memory,
  realtime, review/guardian, and plan state decisions are settled.
- `app-server-implementation-trim`: refactor message processing, outgoing
  messages, request processors, thread history, and event mapping to the final
  smaller protocol.
- `prompt-cleanup`: remove prompt/model-instruction references to deleted
  features after the retained runtime/tool surface is clear.

### Layer 5: final integration

- `cargo-feature-dependency-prune`: remove feature gates, workspace deps, crate
  deps, and lockfile entries that became unreachable after semantic cleanup.
- `tests-rebuild`: rebuild retained tests around auth, Unix-socket app-server,
  retained thread APIs, dynamic tools, local `exec_command`/`write_stdin`,
  `view_image`, hosted web/image gating, rollouts, and thread store.
- `final-checks`: run formatting and Cargo checks/tests only through
  `cargo-modal`, not raw local Cargo.

### Known high-conflict files

Do not assign these files to two workers at once unless their contracts are
explicitly non-overlapping:

- `codex-rs/protocol/src/protocol.rs`
- `codex-rs/app-server/src/message_processor.rs`
- `codex-rs/app-server-protocol/src/protocol/*`
- `codex-rs/core/src/session/mod.rs`
- `codex-rs/core/src/session/session.rs`
- `codex-rs/core/src/session/turn.rs`
- `codex-rs/core/src/tools/spec_plan.rs`
- `codex-rs/config/src/lib.rs`
- `codex-rs/config/src/config_toml.rs`
- `codex-rs/exec/src/cli.rs`

### Pre-compile uncertainties

- Some protocol types may still be needed temporarily as rollout or
  thread-store compatibility glue until the state boundary is deliberately
  broken.
- The first `execpolicy-removal` worker was stopped before merge because it
  began broad automated edits across session/runtime files and left a partial
  worktree. Do not merge `.worktrees/execpolicy-removal` as-is; restart this
  slice from current main or manually salvage reviewed hunks only.
- A second `execpolicy-removal-restart` worker was also stopped before merge:
  it removed config/protocol exports while leaving core session, app-server,
  and test references to execpolicy amendment/cache paths. Do not merge
  `.worktrees/execpolicy-removal-restart` as-is. The next attempt should either
  be a deliberately broad approval/sandbox/runtime slice or a much narrower
  protocol-only amendment removal with all app-server/core call sites handled.
- Approval/sandbox removal may either collapse cleanly into local full access or
  leave short-lived app-server/config compatibility shims that should be removed
  by the integration slices.
- MCP/code-mode removal may expose helpers that should remain for dynamic tools;
  workers must keep dynamic-tool protocol and runtime behavior unless explicitly
  told otherwise.
- Test cleanup may be cheapest as deletion in early slices and reconstruction in
  `tests-rebuild`; do not preserve tests for removed behavior just to keep old
  coverage passing.
- `codex-rs/tools` should remain for now, but may become removable or splittable
  after MCP/code-mode/plugin/request-plugin-install/provider cleanup.
- `CODEX_HOME` naming is partly compatibility surface and partly product
  identity. The retained boundary must not load `~/.codex` by default, but
  renaming every internal symbol may be a later cleanup rather than a blocker.
- Auth/provider cleanup may depend on final app-server initialize/account
  protocol shape.
- The first `auth-provider-simplification` worker was stopped before merge
  because it moved from provider/catalog cleanup into broad automated test
  deletion and partial protocol/model rewrites. Do not merge
  `.worktrees/auth-provider-simplification` as-is; restart with a smaller
  contract, likely one of: Bedrock-only removal, command-auth-only removal, or
  local model catalog pruning.
- Fresh root attempt `semantic-cleanup-20260528-root` launched
  `auth-bedrock-provider`, `execpolicy-narrow`, and `mcp-config-trim` workers,
  but all three provider subprocesses stalled waiting for stdin before making
  worktree edits. They were stopped with no commits and must not be merged as
  worker output. Their transcripts are useful only as discovery.
- `auth-bedrock-provider` discovery showed Bedrock is mostly concentrated in
  `codex-rs/model-provider`, `codex-rs/model-provider-info`, and targeted
  app-server/core tests. A future Bedrock-only slice should remove the built-in
  Bedrock provider/catalog/auth factory and then handle those tests directly,
  while leaving broader account protocol shape to the auth/app-server slices.
- `execpolicy-narrow` discovery confirmed that the crate directory is already
  absent on current main, but execpolicy semantics remain in config
  requirements, core session/state/context, protocol approval amendments,
  app-server warning/amendment mapping, CLI `execpolicy check`, and tests. This
  is no longer a crate-only deletion; the next attempt should be either a
  deliberately broad approval/sandbox/runtime slice or a serial manual edit of
  all amendment/config/runtime call sites.
- `mcp-config-trim` discovery confirmed that MCP config editing/loading is
  entangled with core session refresh, app-server request processors, protocol
  refresh events, and many config tests. Do not attempt a config-only MCP
  deletion unless it is paired with the corresponding core/app-server refresh
  removal, while preserving dynamic tools.
- Root restart after `b32def8fd` completed Wave A on main:
  - `ad8426282` localized git trust-root lookup to direct local filesystem
    inspection and removed the dirty `codex_file_system` dependency from
    `codex-git-utils`; `cargo-modal --repo codex-rs --dirty check -p
    codex-git-utils` passed.
  - `9e3c7aca6` removed stored agent-identity auth records, the login-side
    helper module, tests, and direct `CodexAuth::AgentIdentity` runtime use.
    Targeted `cargo-modal` checks reached existing dependency/protocol fallout
    (`codex-arg0` missing removed helper crates and app-server-protocol stale
    references) before a clean crate result; scoped `rg` searches showed no
    remaining login/model-provider direct agent-identity storage/helper use.
  - `f9d096e23` removed Bedrock provider/catalog/signing code, AWS provider
    config fields, direct Bedrock account/protocol representation, and targeted
    Bedrock tests. Scoped `rg` searches showed no remaining Bedrock/AWS provider
    symbols. `cargo-modal --dirty check -p codex-model-provider-info` and the
    combined model-provider check are blocked by existing
    app-server-protocol fallout, not Bedrock references.
- `cargo-modal` clean Git source mode currently dispatches from `/workspace`
  without the nested `codex-rs/Cargo.toml`; use explicit dirty/worktree upload
  from `codex-rs` until that wrapper behavior is understood.
- The next Wave B work should start serially with
  `execpolicy-approval-runtime`; the existing app-server-protocol compile
  failures are largely stale producer/mapper references for permissions,
  sandbox, MCP, plan/file-change, guardian, and schema/codegen leftovers. Do
  not patch those by reintroducing protocol types.
- A first `execpolicy-approval-runtime` Wave B worker was launched after Wave A
  but stalled waiting for provider stdin before making edits. Its useful
  discovery: current main already lacks `codex-rs/core/src/exec_policy.rs`, so
  remaining work is in config requirements/state, core session/tool runtime,
  app-server routing/mapping, CLI commands/tests, and stale
  app-server-protocol references. Restart this slice from current main; do not
  merge `.worktrees/execpolicy-approval-runtime-wave-b` as worker output.
- A restarted `execpolicy-approval-runtime-b2` worker also stalled waiting for
  provider stdin. Root salvaged and reviewed only the coherent local unified
  exec portion as `8ec0c3c60`: model-visible unified exec approval/sandbox
  permission arguments were removed, local process launch now bypasses the
  removed orchestrator/runtime approval path, and stale unified-exec tests for
  exec-server/sandbox/network approval were deleted or adapted. Do not merge
  the uncommitted config edits left in
  `.worktrees/execpolicy-approval-runtime-wave-b2`; they silently discarded
  `[rules]` requirements by binding them to `_` while leaving TOML parsing and
  tests behind. Remaining execpolicy work should handle config requirements,
  core session/state/context, CLI `execpolicy check`, app-server warning and
  amendment mapping, and tests as a coherent follow-up rather than ignoring
  parsed policy data.
- The follow-up `execpolicy-config-session-followup` worker was stopped after
  another provider-stdin stall and must not be merged. Its only diff edited
  `codex-rs/config/src/config_requirements.rs` by removing some execpolicy and
  permission-profile fields but then bound `allowed_sandbox_modes` to `_`,
  silently discarding managed sandbox requirements while leaving adjacent
  config surfaces and tests unresolved. Restart config/session/CLI execpolicy
  cleanup as a more deliberate serial root patch or a smaller worker whose
  contract explicitly removes or preserves each managed config field rather
  than dropping parsed data.

### Daemex sandbox CLI copy follow-up

The narrow daemex-derived copy slice deletes
`codex-rs/protocol/src/request_permissions.rs` and
`codex-rs/utils/cli/src/sandbox_mode_cli_arg.rs`, then removes the direct
module/export and CLI wrapper references.

Remaining `request_permissions` hits after that slice are intentionally broader
semantic follow-up across core session/protocol/app-server/tests and the
permissions pipeline. Do not treat them as part of the literal copy slice.

Remaining `--sandbox` hits after that slice are broader sandbox-policy docs and
tests outside the wrapper cleanup. Handle them in the later sandbox /
approvals / permissions semantic removal, likely using daemex as the reference.

### Cargo-modal observations

- Targeted `cargo-modal --dirty check -p codex-login -p codex-cli` after the
  access-token auth removal failed before reaching the edited crates because
  `codex-protocol` is already inconsistent with earlier removals:
  `request_permissions` imports remain after the module was removed,
  `protocol/src/network_policy.rs` still imports the removed
  `codex_network_proxy` crate, and `protocol/src/config_types.rs` still has a
  schema helper that references removed schema/codegen types. These blockers
  belong to the protocol/sandbox-permissions/dependency cleanup, not to the
  access-token auth slice.
- Root follow-up fixed the immediate `codex-protocol` blockers and committed
  `2a94c1338`; `cargo-modal --dirty --cache none check -p codex-protocol`
  passed afterward.
- Root follow-up removed access-token login entry points and committed
  `9f4f93d46`. `CODEX_ACCESS_TOKEN`, `login --with-access-token`, and the
  access-token stdin helper are gone from `codex-rs`.
- Root follow-up committed `45a2dce04` for leftover syntax/API fragments:
  dangling app-server-protocol attributes were removed and Responses API
  structured-output text formatting now serializes `type = "json_schema"` with
  the retained `strict` field.
- The next targeted `cargo-modal --dirty check -p codex-login -p codex-cli`
  gets past those parse fragments but still fails in broader stale surfaces:
  `codex-git-utils` imports the removed `codex_file_system` crate; app-server
  protocol still references removed `codex_shell_command`,
  `RequestPermissions` guardian/action variants, generated schema helper types,
  removed server notifications (`PlanDelta`, `FileChangePatchUpdated`,
  guardian approval review events), and v2 re-exports for permission/sandbox/MCP
  types. Treat this as the next serial protocol/runtime cleanup frontier, not as
  an auth-login regression.
- Root follow-up committed `08b1e01a8` for the app-server-protocol portion of
  that frontier, but this was the wrong direction: it reintroduced/kept
  app-server notifications and item structures for removed surfaces
  (`PlanDelta`, `FileChangePatchUpdated`, guardian approval review events),
  moved removed concepts through `codex_protocol` to satisfy compilation, and
  degraded parsed command actions to `Unknown`. It was reverted by
  `b32def8fd`. Future app-server-protocol work must be scheduled after the
  upstream producers are removed and must delete stale surfaces instead of
  compile-shimming them.
- After reverting `08b1e01a8`, the stopped root left dirty git-utils edits that
  remove the async `codex_file_system` dependency from
  `codex-rs/git-utils/src/info.rs` and `lib.rs`. These edits may be the right
  local-only direction, but they are uncommitted and need a focused review or
  fresh worker slice before inclusion.
