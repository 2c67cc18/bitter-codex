# Thread Title Reconciliation

## Upstream Reference

- `3cdce52865` - preserve renamed thread titles during reconciliation

Classification: `manual-port`.

Reason: rollout reconciliation, SQLite state metadata, and thread-store title
projection survive locally, but the upstream patch sits beside adjacent
memory-mode/test churn and should be adapted to this fork's state/runtime
layout.

## Goal

Keep explicit user-renamed thread titles when rollout reconciliation or
backfill observes an inferred title derived from the first user message.

## Behavior

- detect when an existing SQLite title is explicit rather than first-message
  derived
- during rollout reconciliation/backfill, preserve that explicit title instead
  of replacing it with the rollout-derived title
- keep existing stale inferred-title repair behavior
- avoid session-index scans during startup backfill

## Tests

Port or adapt upstream coverage only where it maps to surviving behavior:

- `3cdce52865`: renamed SQLite titles survive rollout reconciliation
- `3cdce52865`: inferred titles can still be repaired from rollout metadata
- `3cdce52865`: title preservation is covered in `codex-state` object equality
  tests where practical

## Validation

Run after implementation:

- `cargo +stable fmt`
- `cargo +stable test -p codex-state`
- `cargo +stable test -p codex-rollout`
- because this touches state/rollout metadata used by core resume flows, run
  `cargo +stable test --workspace` after focused tests pass
