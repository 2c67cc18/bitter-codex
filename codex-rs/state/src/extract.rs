use crate::model::ThreadMetadata;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::protocol::SessionMetaLine;
use codex_protocol::protocol::TurnContextItem;
use serde::Serialize;
use serde_json::Value;

const IMAGE_ONLY_USER_MESSAGE_PLACEHOLDER: &str = "[Image]";
const USER_MESSAGE_BEGIN: &str = "## My request for Codex:";

pub fn apply_rollout_item(
    metadata: &mut ThreadMetadata,
    item: &RolloutItem,
    default_provider: &str,
) {
    match item {
        RolloutItem::SessionMeta(meta_line) => apply_session_meta_from_item(metadata, meta_line),
        RolloutItem::TurnContext(turn_ctx) => apply_turn_context(metadata, turn_ctx),
        RolloutItem::EventMsg(event) => apply_event_msg(metadata, event),
        RolloutItem::ResponseItem(item) => apply_response_item(metadata, item),
        RolloutItem::Compacted(_) => {}
    }
    if metadata.model_provider.is_empty() {
        metadata.model_provider = default_provider.to_string();
    }
}

pub fn rollout_item_affects_thread_metadata(item: &RolloutItem) -> bool {
    match item {
        RolloutItem::SessionMeta(_) | RolloutItem::TurnContext(_) => true,
        RolloutItem::EventMsg(EventMsg::TokenCount(_)) => true,
        RolloutItem::ResponseItem(item) => user_response_item_preview(item).is_some(),
        RolloutItem::EventMsg(_) | RolloutItem::Compacted(_) => false,
    }
}

fn apply_session_meta_from_item(metadata: &mut ThreadMetadata, meta_line: &SessionMetaLine) {
    if metadata.id != meta_line.meta.id {
        return;
    }
    metadata.id = meta_line.meta.id;
    metadata.source = enum_to_string(&meta_line.meta.source);
    if let Some(provider) = meta_line.meta.model_provider.as_deref() {
        metadata.model_provider = provider.to_string();
    }
    if !meta_line.meta.cli_version.is_empty() {
        metadata.cli_version = meta_line.meta.cli_version.clone();
    }
    if !meta_line.meta.cwd.as_os_str().is_empty() {
        metadata.cwd = meta_line.meta.cwd.clone();
    }
    if let Some(git) = meta_line.git.as_ref() {
        metadata.git_sha = git.commit_hash.as_ref().map(|sha| sha.0.clone());
        metadata.git_branch = git.branch.clone();
        metadata.git_origin_url = git.repository_url.clone();
    }
}

fn apply_turn_context(metadata: &mut ThreadMetadata, turn_ctx: &TurnContextItem) {
    if metadata.cwd.as_os_str().is_empty() {
        metadata.cwd = turn_ctx.cwd.clone();
    }
    metadata.model = Some(turn_ctx.model.clone());
    metadata.reasoning_effort = turn_ctx.effort;
}

fn apply_event_msg(metadata: &mut ThreadMetadata, event: &EventMsg) {
    match event {
        EventMsg::TokenCount(token_count) => {
            if let Some(info) = token_count.info.as_ref() {
                metadata.tokens_used = info.total_token_usage.total_tokens.max(0);
            }
        }
        _ => {}
    }
}

fn apply_response_item(metadata: &mut ThreadMetadata, item: &ResponseItem) {
    let Some((preview, title)) = user_response_item_preview_and_title(item) else {
        return;
    };
    if metadata.first_user_message.is_none() {
        metadata.first_user_message = Some(preview.clone());
    }
    set_preview_if_empty(metadata, Some(preview));
    if metadata.title.is_empty() && !title.is_empty() {
        metadata.title = title;
    }
}

fn set_preview_if_empty(metadata: &mut ThreadMetadata, preview: Option<String>) {
    if metadata.preview.is_none() {
        metadata.preview = preview;
    }
}

fn strip_user_message_prefix(text: &str) -> &str {
    match text.find(USER_MESSAGE_BEGIN) {
        Some(idx) => text[idx + USER_MESSAGE_BEGIN.len()..].trim(),
        None => text.trim(),
    }
}

fn user_response_item_preview(item: &ResponseItem) -> Option<String> {
    user_response_item_preview_and_title(item).map(|(preview, _)| preview)
}

fn user_response_item_preview_and_title(item: &ResponseItem) -> Option<(String, String)> {
    let ResponseItem::Message { role, content, .. } = item else {
        return None;
    };
    if role != "user" {
        return None;
    }
    let mut text = String::new();
    let mut has_image = false;
    for item in content {
        match item {
            ContentItem::InputText { text: part } => text.push_str(part),
            ContentItem::InputImage { .. } => has_image = true,
            ContentItem::OutputText { .. } => {}
        }
    }
    let title = strip_user_message_prefix(text.as_str()).to_string();
    if !title.is_empty() {
        Some((title.clone(), title))
    } else if has_image {
        Some((
            IMAGE_ONLY_USER_MESSAGE_PLACEHOLDER.to_string(),
            String::new(),
        ))
    } else {
        None
    }
}

pub(crate) fn enum_to_string<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(s)) => s,
        Ok(other) => other.to_string(),
        Err(_) => String::new(),
    }
}
