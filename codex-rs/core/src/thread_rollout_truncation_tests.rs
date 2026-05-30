use super::*;
use codex_protocol::models::ContentItem;
use pretty_assertions::assert_eq;

fn message(role: &str, text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: role.to_string(),
        content: vec![ContentItem::InputText {
            text: text.to_string(),
        }],
        phase: None,
    }
}

#[test]
fn truncates_rollout_to_last_n_fork_turns_drops_startup_prefix_even_when_under_limit() {
    let rollout = vec![
        RolloutItem::ResponseItem(message("developer", "startup developer context")),
        RolloutItem::ResponseItem(message("user", "current task")),
        RolloutItem::ResponseItem(message("assistant", "answer")),
    ];

    let truncated = truncate_rollout_to_last_n_fork_turns(&rollout, 2);
    let expected = rollout[1..].to_vec();

    assert_eq!(
        serde_json::to_value(&truncated).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
}
