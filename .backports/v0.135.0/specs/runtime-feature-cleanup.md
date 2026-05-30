# Runtime Feature Cleanup

## Goal

Remove runtime feature flags that no longer need to be configurable in this
fork.

## Remove

- `shell_zsh_fork`
- `enable_request_compression`
- `fast_mode`

## Behavior

`shell_zsh_fork`:

- remove the feature enum/spec entry
- remove config parsing/lock output for the feature
- stop forcing zsh through `zsh_path`
- use `user_shell_override` when present
- otherwise use `default_user_shell()`

`enable_request_compression`:

- remove the feature enum/spec entry
- always enable request compression
- remove config/lock output for the feature

`fast_mode`:

- remove the feature enum/spec entry
- keep `service_tier = "fast"` accepted when the model supports it
- remove config/lock output for the feature

## Keep

Keep `runtime_metrics`.

It is still used by OTEL, not by the removed analytics crate. It controls
whether the OTEL metrics client installs the runtime metrics reader and enables
runtime summary collection.

Keep `responses_websocket_response_processed` default false for now.

It gates `response.processed` after normal streamed Responses turns and should
also gate the same notification after restored remote compaction v2.

## Tests

Update or remove tests that assert these feature keys exist. Add new coverage
only if the cleanup changes behavior in a way existing tests do not cover.

## Validation

Run after implementation:

- `cargo +stable fmt`
- `cargo +stable test -p codex-features`
- `cargo +stable test -p codex-config`
- `cargo +stable test -p codex-core`
