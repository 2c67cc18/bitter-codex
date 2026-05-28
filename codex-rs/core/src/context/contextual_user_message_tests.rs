use super::*;
use crate::context::ContextualUserFragment;
use crate::context::TurnAborted;
use pretty_assertions::assert_eq;

#[test]
fn detects_environment_context_fragment() {
    assert!(is_contextual_user_fragment(&ContentItem::InputText {
        text: "<environment_context>\n<cwd>/tmp</cwd>\n</environment_context>".to_string(),
    }));
}

#[test]
fn contextual_user_fragment_is_dyn_compatible() {
    let fragment: Box<dyn ContextualUserFragment> = Box::new(TurnAborted);

    assert_eq!(fragment.render(), "<turn_aborted />");
}

#[test]
fn ignores_regular_user_text() {
    assert!(!is_contextual_user_fragment(&ContentItem::InputText {
        text: "hello".to_string(),
    }));
}
