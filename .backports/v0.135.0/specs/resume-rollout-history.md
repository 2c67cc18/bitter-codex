# Resume / Rollout History

## Upstream References

- `5cd9b8086` - respect resume cwd overrides for idle cached threads
- `61cbf3574` - drop startup context when truncating forked rollouts

Classification: `manual-port`.

Reason: both patches touch surviving behavior, but neither applies cleanly
because local tests and surrounding app-server/core layout differ.

## Goal

Port the surviving resume and fork-rollout history fixes.

## Behavior

Resume overrides:

- when a loaded thread is idle, unsubscribed, and has no active work, treat it
  as cache if resume overrides differ
- unload the cached thread so cold resume rebuilds it with requested cwd/config
- preserve rejoin behavior for subscribed or running threads

Fork rollout truncation:

- truncate bounded fork history from the first fork-turn boundary when fewer
  than the requested number of fork turns exist
- drop pre-turn startup context in that case
- keep forked children able to rebuild context after prefix truncation

## Tests

Port or adapt upstream coverage only where it maps to surviving behavior:

- `5cd9b8086`: idle cached thread resume honors cwd/config overrides
- `5cd9b8086`: subscribed or running thread resume still rejoins existing work
- `61cbf3574`: fork truncation drops startup prefix under the requested limit
- `61cbf3574`: bounded fork spawn rebuilds context after prefix truncation

## Validation

Run after implementation:

- `cargo +stable fmt`
- `cargo +stable test -p codex-app-server`
- `cargo +stable test -p codex-core thread_rollout_truncation`
