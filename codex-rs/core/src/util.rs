use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use codex_login::auth_env_telemetry::AuthEnvTelemetry;
use rand::Rng;
use tracing::error;

const INITIAL_DELAY_MS: u64 = 200;
const BACKOFF_FACTOR: f64 = 2.0;

/// Emit structured feedback metadata as key/value pairs.
///
/// This logs a tracing event with `target: "feedback_tags"`. Feedback collectors can capture these
/// fields and later attach them as tags when feedback is uploaded.
///
/// Values are wrapped with [`tracing::field::DebugValue`], so the expression only needs to
/// implement [`std::fmt::Debug`].
///
/// Example:
///
/// ```rust
/// codex_core::feedback_tags!(model = "gpt-5", cached = true);
/// codex_core::feedback_tags!(provider = provider_id, request_id = request_id);
/// ```
#[macro_export]
macro_rules! feedback_tags {
    ($( $key:ident = $value:expr ),+ $(,)?) => {
        ::tracing::info!(
            target: "feedback_tags",
            $( $key = ::tracing::field::debug(&$value) ),+
        );
    };
}

struct Auth401FeedbackSnapshot<'a> {
    request_id: &'a str,
    cf_ray: &'a str,
    error: &'a str,
    error_code: &'a str,
}

impl<'a> Auth401FeedbackSnapshot<'a> {
    fn from_optional_fields(
        request_id: Option<&'a str>,
        cf_ray: Option<&'a str>,
        error: Option<&'a str>,
        error_code: Option<&'a str>,
    ) -> Self {
        Self {
            request_id: request_id.unwrap_or(""),
            cf_ray: cf_ray.unwrap_or(""),
            error: error.unwrap_or(""),
            error_code: error_code.unwrap_or(""),
        }
    }
}

pub(crate) struct FeedbackRequestTags<'a> {
    pub(crate) endpoint: &'a str,
    pub(crate) auth_header_attached: bool,
    pub(crate) auth_header_name: Option<&'a str>,
    pub(crate) auth_mode: Option<&'a str>,
    pub(crate) auth_retry_after_unauthorized: Option<bool>,
    pub(crate) auth_recovery_mode: Option<&'a str>,
    pub(crate) auth_recovery_phase: Option<&'a str>,
    pub(crate) auth_connection_reused: Option<bool>,
    pub(crate) auth_request_id: Option<&'a str>,
    pub(crate) auth_cf_ray: Option<&'a str>,
    pub(crate) auth_error: Option<&'a str>,
    pub(crate) auth_error_code: Option<&'a str>,
    pub(crate) auth_recovery_followup_success: Option<bool>,
    pub(crate) auth_recovery_followup_status: Option<u16>,
}

pub(crate) fn emit_feedback_request_tags_with_auth_env(
    tags: &FeedbackRequestTags<'_>,
    auth_env: &AuthEnvTelemetry,
) {
    let auth_request_id = tags.auth_request_id.unwrap_or("");
    let auth_cf_ray = tags.auth_cf_ray.unwrap_or("");
    let auth_error = tags.auth_error.unwrap_or("");
    let auth_error_code = tags.auth_error_code.unwrap_or("");
    let provider_env_key_name = auth_env.provider_env_key_name.as_deref().unwrap_or("");
    let auth_header_name = tags.auth_header_name.unwrap_or("");
    let auth_mode = tags.auth_mode.unwrap_or("");
    let auth_recovery_mode = tags.auth_recovery_mode.unwrap_or("");
    let auth_recovery_phase = tags.auth_recovery_phase.unwrap_or("");
    feedback_tags!(
        endpoint = tags.endpoint,
        auth_header_attached = tags.auth_header_attached,
        auth_header_name = auth_header_name,
        auth_mode = auth_mode,
        auth_retry_after_unauthorized = tags.auth_retry_after_unauthorized,
        auth_recovery_mode = auth_recovery_mode,
        auth_recovery_phase = auth_recovery_phase,
        auth_connection_reused = tags.auth_connection_reused,
        auth_request_id = auth_request_id,
        auth_cf_ray = auth_cf_ray,
        auth_error = auth_error,
        auth_error_code = auth_error_code,
        auth_recovery_followup_success = tags.auth_recovery_followup_success,
        auth_recovery_followup_status = tags.auth_recovery_followup_status,
        openai_api_key_env_present = auth_env.openai_api_key_env_present,
        codex_api_key_env_present = auth_env.codex_api_key_env_present,
        codex_api_key_env_enabled = auth_env.codex_api_key_env_enabled,
        provider_env_key_name = provider_env_key_name,
        provider_env_key_present = auth_env.provider_env_key_present,
        refresh_token_url_override_present = auth_env.refresh_token_url_override_present,
    );
}

pub(crate) fn emit_feedback_auth_recovery_tags(
    auth_recovery_mode: &str,
    auth_recovery_phase: &str,
    auth_recovery_outcome: &str,
    auth_request_id: Option<&str>,
    auth_cf_ray: Option<&str>,
    auth_error: Option<&str>,
    auth_error_code: Option<&str>,
) {
    let auth_401 = Auth401FeedbackSnapshot::from_optional_fields(
        auth_request_id,
        auth_cf_ray,
        auth_error,
        auth_error_code,
    );
    feedback_tags!(
        auth_recovery_mode = auth_recovery_mode,
        auth_recovery_phase = auth_recovery_phase,
        auth_recovery_outcome = auth_recovery_outcome,
        auth_401_request_id = auth_401.request_id,
        auth_401_cf_ray = auth_401.cf_ray,
        auth_401_error = auth_401.error,
        auth_401_error_code = auth_401.error_code
    );
}

pub fn backoff(attempt: u64) -> Duration {
    let exp = BACKOFF_FACTOR.powi(attempt.saturating_sub(1) as i32);
    let base = (INITIAL_DELAY_MS as f64 * exp) as u64;
    let jitter = rand::rng().random_range(0.9..1.1);
    Duration::from_millis((base as f64 * jitter) as u64)
}

pub(crate) fn error_or_panic(message: impl std::string::ToString) {
    if cfg!(debug_assertions) {
        panic!("{}", message.to_string());
    } else {
        error!("{}", message.to_string());
    }
}

pub fn resolve_path(base: &Path, path: &PathBuf) -> PathBuf {
    if path.is_absolute() {
        path.clone()
    } else {
        base.join(path)
    }
}

/// Trim a thread name and return `None` if it is empty after trimming.
pub fn normalize_thread_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
#[path = "util_tests.rs"]
mod tests;
