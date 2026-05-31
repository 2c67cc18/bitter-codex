use codex_protocol::models::ContentItem;

use super::AdditionalContextUserFragment;
use super::EnvironmentContext;
use super::FragmentRegistration;
use super::FragmentRegistrationProxy;
use super::TurnAborted;

static ENVIRONMENT_CONTEXT_REGISTRATION: FragmentRegistrationProxy<EnvironmentContext> =
    FragmentRegistrationProxy::new();
static TURN_ABORTED_REGISTRATION: FragmentRegistrationProxy<TurnAborted> =
    FragmentRegistrationProxy::new();
static ADDITIONAL_CONTEXT_REGISTRATION: FragmentRegistrationProxy<AdditionalContextUserFragment> =
    FragmentRegistrationProxy::new();
static CONTEXTUAL_USER_FRAGMENTS: &[&dyn FragmentRegistration] = &[
    &ENVIRONMENT_CONTEXT_REGISTRATION,
    &TURN_ABORTED_REGISTRATION,
    &ADDITIONAL_CONTEXT_REGISTRATION,
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
