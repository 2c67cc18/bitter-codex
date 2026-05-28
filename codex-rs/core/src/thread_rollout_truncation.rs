use crate::context_manager::is_user_turn_boundary;
use crate::event_mapping;
use codex_protocol::items::TurnItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::InitialHistory;
use codex_protocol::protocol::RolloutItem;

pub(crate) fn initial_history_has_prior_user_turns(conversation_history: &InitialHistory) -> bool {
    conversation_history.scan_rollout_items(rollout_item_is_user_turn_boundary)
}

fn rollout_item_is_user_turn_boundary(item: &RolloutItem) -> bool {
    match item {
        RolloutItem::ResponseItem(item) => is_user_turn_boundary(item),
        _ => false,
    }
}

pub(crate) fn user_message_positions_in_rollout(items: &[RolloutItem]) -> Vec<usize> {
    let mut user_positions = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        match item {
            RolloutItem::ResponseItem(item @ ResponseItem::Message { .. })
                if matches!(
                    event_mapping::parse_turn_item(item),
                    Some(TurnItem::UserMessage(_))
                ) =>
            {
                user_positions.push(idx);
            }
            _ => {}
        }
    }
    user_positions
}

pub(crate) fn fork_turn_positions_in_rollout(items: &[RolloutItem]) -> Vec<usize> {
    let mut fork_turn_positions = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        if let RolloutItem::ResponseItem(item) = item
            && is_real_user_message_boundary(item)
        {
            fork_turn_positions.push(idx);
        }
    }
    fork_turn_positions
}

pub(crate) fn truncate_rollout_before_nth_user_message_from_start(
    items: &[RolloutItem],
    n_from_start: usize,
) -> Vec<RolloutItem> {
    if n_from_start == usize::MAX {
        return items.to_vec();
    }

    let user_positions = user_message_positions_in_rollout(items);

    if user_positions.len() <= n_from_start {
        return items.to_vec();
    }

    let cut_idx = user_positions[n_from_start];
    items[..cut_idx].to_vec()
}

pub(crate) fn truncate_rollout_to_last_n_fork_turns(
    items: &[RolloutItem],
    n_from_end: usize,
) -> Vec<RolloutItem> {
    if n_from_end == 0 {
        return Vec::new();
    }

    let fork_turn_positions = fork_turn_positions_in_rollout(items);
    if fork_turn_positions.len() <= n_from_end {
        return items.to_vec();
    }

    let keep_idx = fork_turn_positions[fork_turn_positions.len() - n_from_end];
    items[keep_idx..].to_vec()
}

fn is_real_user_message_boundary(item: &ResponseItem) -> bool {
    matches!(
        event_mapping::parse_turn_item(item),
        Some(TurnItem::UserMessage(_))
    )
}
