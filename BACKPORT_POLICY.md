# Backport Policy

This fork tracks selected Rust-side fixes from upstream `openai/codex`. It does
not mirror upstream wholesale. Path overlap is not enough to make a commit a
good cherry-pick.

## Scope

Prefer backports that improve behavior in surfaces that still exist in this
repo:

- `codex-rs/core`
- `codex-rs/protocol`
- `codex-rs/app-server-protocol`
- `codex-rs/app-server`
- `codex-rs/config`
- `codex-rs/state`
- `codex-rs/tools`
- `codex-rs/codex-api`
- small shared utilities under `codex-rs/utils`

Excluded surfaces:

- TUI
- SDKs
- Bazel
- release packaging
- Windows sandbox
- daemon / remote-control-only plumbing
- plugins / extensions / hosted app inventory
- memories
- vendor trees
- GitHub workflows
- repository maintenance

## Classification

For every upstream commit under consideration, classify it before attempting a
port.

- `cherry-pick`: the changed files and their dependencies exist here, and the
  upstream patch applies with only normal conflict resolution.
- `manual-port`: useful behavior, but the patch is shaped by dependencies,
  generated files, lockfile churn, or removed surfaces.
- `skip`: the commit primarily serves a removed surface, or its value depends on
  a larger feature we are not carrying.
- `defer`: maybe useful, but needs a design decision or adjacent commits.

Path overlap alone must not determine the classification. Check the commit
message, patch body, new crates/modules, tests, and feature gates.

Group related commits into bundles when they change the same behavior, rely on
each other, or are better reviewed as one local change.

Order bundles and commits by upstream chronology. This is not fundamental, but
it keeps the backport history easier to compare with upstream. When a commit
has a direct prerequisite, keep the prerequisite first.

For multi-bundle backport passes, include a small DAG in the pass document. The
DAG should show cross-bundle dependencies, parallel lanes, and a reasonable
serial order.

Port or adapt the upstream test coverage when it applies to surviving behavior.
Do not copy tests for excluded surfaces.

## Upstream Anchor

Backports should stay anchored to the upstream commits being ported.

- Specs should tie behavior and tests to specific upstream commits.
- Do not add behavior or tests beyond those commits unless local adaptation
  requires it.
- When local adaptation is required, say why in the spec or commit body.

## Commit Shape

Clean cherry-picks should keep the upstream commit author and message. Include
the original upstream commit SHA in the local commit message.

If a cherry-pick does not apply cleanly enough to preserve the upstream patch,
turn it into a manual port and document the upstream commit SHA in the commit
body.

Manual-port bundle commits should mention every upstream commit they port in the
commit body.

Manual-port commits should include `Co-authored-by` trailers for the original
authors of the upstream commits they port, plus the local implementer.

## Manual Ports

Use manual ports for dependency upgrades and schema/runtime adaptations. Do not
cherry-pick these directly just because upstream has a commit.

Manual-port bundles need a short spec under `.backports/<tag>/specs/` before
implementation.
The spec should list upstream commits, surviving behavior, excluded surfaces,
upstream-anchored tests to port or adapt, and validation commands.

Examples:

- SQLx / SQLite bumps: update locally; let compile errors drive source edits.
- Generated schemas: regenerate locally.
- Lockfiles: update with local Cargo commands.
- Cross-cutting refactors: port only surviving modules.

## Review Checklist

Before taking a commit:

1. Identify the upstream tag and commit SHA.
2. List removed surfaces touched by the commit.
3. List surviving surfaces touched by the commit.
4. Decide `cherry-pick`, `manual-port`, `skip`, or `defer`.
5. Port or adapt applicable tests.
6. If the commit is a dependency bump, explain the runtime/security reason and
   perform it manually.

After taking a Rust change, run `cargo +stable fmt` in `codex-rs`, then focused
package tests. If `common`, `core`, or `protocol` changed, run workspace tests
after focused tests pass.
