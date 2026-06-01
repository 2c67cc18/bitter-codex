# Shell Runtime Cleanup

## Upstream References

- `9152ebd28` - preserve shell cleanup on interruption

Classification: `manual-port`.

Reason: shell execution, process cleanup, and local sandbox policy behavior
survive, but upstream changed larger tool-handler and runtime modules that have
different local structure.

## Goal

Port the surviving shell runtime cleanup fix without restoring removed tool
surfaces.

## Behavior

Shell cleanup:

- preserve shell cleanup behavior when a turn or shell task is interrupted
- keep process-group cleanup and child termination behavior aligned with the
  upstream interruption fix
- adapt the upstream parallel-tool cleanup behavior to the local tool router

## Adapt Out

Do not restore or copy:

- removed tool handlers
- Windows sandbox runner plumbing
- code-mode or multi-agent test surfaces
- upstream shell helpers that do not exist locally

## Tests

Port or adapt upstream coverage only where it maps to surviving behavior:

- `9152ebd28`: interrupted shell execution still runs cleanup paths
- `9152ebd28`: process groups are cleaned up after interruption

## Validation

Run after implementation:

- `cargo +stable fmt`
- `cargo +stable test -p codex-core exec`
- `cargo +stable test -p codex-core tools`
- `cargo +stable test -p codex-core approvals`
