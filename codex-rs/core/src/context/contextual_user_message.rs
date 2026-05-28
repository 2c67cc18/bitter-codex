use codex_protocol::models::ContentItem;

use super::EnvironmentContext;
use super::FragmentRegistration;
use super::FragmentRegistrationProxy;
use super::LegacyModelMismatchWarning;
use super::LegacyUnifiedExecProcessLimitWarning;
use super::TurnAborted;
use super::UserShellCommand;

static ENVIRONMENT_CONTEXT_REGISTRATION: FragmentRegistrationProxy<EnvironmentContext> =
    FragmentRegistrationProxy::new();
static USER_SHELL_COMMAND_REGISTRATION: FragmentRegistrationProxy<UserShellCommand> =
    FragmentRegistrationProxy::new();
static TURN_ABORTED_REGISTRATION: FragmentRegistrationProxy<TurnAborted> =
    FragmentRegistrationProxy::new();
static LEGACY_UNIFIED_EXEC_PROCESS_LIMIT_WARNING_REGISTRATION: FragmentRegistrationProxy<
    LegacyUnifiedExecProcessLimitWarning,
> = FragmentRegistrationProxy::new();
static LEGACY_MODEL_MISMATCH_WARNING_REGISTRATION: FragmentRegistrationProxy<
    LegacyModelMismatchWarning,
> = FragmentRegistrationProxy::new();

static CONTEXTUAL_USER_FRAGMENTS: &[&dyn FragmentRegistration] = &[
    &ENVIRONMENT_CONTEXT_REGISTRATION,
    &USER_SHELL_COMMAND_REGISTRATION,
    &TURN_ABORTED_REGISTRATION,
    &LEGACY_UNIFIED_EXEC_PROCESS_LIMIT_WARNING_REGISTRATION,
    &LEGACY_MODEL_MISMATCH_WARNING_REGISTRATION,
];

fn is_standard_contextual_user_text(text: &str) -> bool {
    CONTEXTUAL_USER_FRAGMENTS
        .iter()
        .any(|fragment| fragment.matches_text(text))
}

pub(crate) fn is_contextual_user_fragment(content_item: &ContentItem) -> bool {
    let ContentItem::InputText { text } = content_item else {
        return false;
    };
    is_standard_contextual_user_text(text)
}

#[cfg(test)]
#[path = "contextual_user_message_tests.rs"]
mod tests;
