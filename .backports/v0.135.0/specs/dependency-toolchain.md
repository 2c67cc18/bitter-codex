# Dependency / Toolchain

## Upstream References

- `cca1e0ba1` - Rust toolchain pin update
- `379511dce` - SQLx / bundled SQLite bump

Classification: `manual-port`.

Reason: upstream changes include CI, Bazel, packaging, doctor, and unrelated
touch-ups. The local port should update only the surviving Rust workspace
surface.

## Goal

Port the toolchain and SQLite dependency updates needed by this fork.

## Behavior

Rust toolchain:

- update `codex-rs/rust-toolchain.toml` from `1.93.0` to upstream `1.95.0`
- do not port upstream CI, Bazel, or packaging file changes

SQLx / bundled SQLite:

- update workspace SQLx from `0.8.6` to `0.9.0`
- use upstream SQLx 0.9 feature names
- update local lockfile through Cargo
- adapt `codex-state` query/migrator code for SQLx 0.9
- do not port memory-state or Bazel lock changes for removed surfaces

## Tests

Port or adapt upstream coverage only where it maps to surviving behavior:

- `379511dce`: `codex-state` builds and tests with SQLx 0.9
- `379511dce`: runtime migrators preserve SQLx 0.9 fields
- `379511dce`: dynamic state queries use SQLx 0.9-compatible builders

## Validation

Run after implementation:

- `cargo +stable fmt`
- `cargo +stable test -p codex-state`
- `cargo +stable check -p codex-state --tests`
