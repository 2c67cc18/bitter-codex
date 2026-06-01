# Auth Refresh

## Upstream References

- `6111791d0` - treat `refresh_token_reused` 400s as relogin-required
- `e5afe5bf8` - refresh near-expiry ChatGPT access tokens before requests

Classification: `manual-port`.

Reason: both patches map to the surviving `codex-login` auth manager. They
should be ported together because they touch the same refresh decision and
failure-classification path.

## Goal

Port upstream ChatGPT token refresh robustness fixes.

## Behavior

Refresh-token failure classification:

- classify known terminal refresh-token failures as permanent even when the
  backend returns HTTP 400
- preserve the relogin-required message for `refresh_token_reused`
- avoid retrying a known permanent failure and replacing it with a generic
  transient/cloud-requirements error

Near-expiry refresh:

- proactively refresh managed ChatGPT access tokens inside the upstream
  five-minute expiry window
- keep expired-token refresh behavior
- leave API-key, external-token, and agent identity auth modes unchanged

## Tests

Port or adapt upstream coverage only where it maps to surviving behavior:

- `6111791d0`: HTTP 400 `refresh_token_reused` is treated as permanent and not
  retried
- `e5afe5bf8`: access token expiring within five minutes refreshes
- `e5afe5bf8`: access token outside the refresh window does not refresh

## Validation

Run after implementation:

- `cargo +stable fmt`
- `cargo +stable test -p codex-login auth_refresh`
