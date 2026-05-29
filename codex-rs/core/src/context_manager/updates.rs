use crate::context::ContextualUserFragment;
use crate::context::EnvironmentContext;
use crate::context::ModelSwitchInstructions;
use crate::session::PreviousTurnSettings;
use crate::session::turn_context::TurnContext;
use crate::shell::Shell;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::protocol::TurnContextItem;

fn build_environment_update_item(
    previous: Option<&TurnContextItem>,
    next: &TurnContext,
    shell: &Shell,
) -> Option<ResponseItem> {
    if !next.config.include_environment_context {
        return None;
    }

    let prev = previous?;
    let prev_context = EnvironmentContext::from_turn_context_item(prev, shell.name().to_string());
    let next_context = EnvironmentContext::from_turn_context(next, shell);
    if prev_context.equals_except_shell(&next_context) {
        return None;
    }

    Some(ContextualUserFragment::into(
        EnvironmentContext::diff_from_turn_context_item(prev, &next_context),
    ))
}

fn model_instructions_from_info(model_info: &ModelInfo) -> Option<String> {
    model_info
        .model_messages
        .as_ref()
        .filter(|message| !message.is_empty())
        .map(|_| model_info.get_model_instructions())
}

pub(crate) fn build_model_instructions_update_item(
    previous_turn_settings: Option<&PreviousTurnSettings>,
    next: &TurnContext,
) -> Option<String> {
    let previous_turn_settings = previous_turn_settings?;
    if previous_turn_settings.model == next.model_info.slug {
        return None;
    }
    let model_instructions = model_instructions_from_info(&next.model_info)?;

    if model_instructions.is_empty() {
        return None;
    }

    Some(ModelSwitchInstructions::new(model_instructions).render())
}

pub(crate) fn build_developer_update_item(text_sections: Vec<String>) -> Option<ResponseItem> {
    build_text_message("developer", text_sections)
}

pub(crate) fn build_contextual_user_message(text_sections: Vec<String>) -> Option<ResponseItem> {
    build_text_message("user", text_sections)
}

fn build_text_message(role: &str, text_sections: Vec<String>) -> Option<ResponseItem> {
    if text_sections.is_empty() {
        return None;
    }

    let content = text_sections
        .into_iter()
        .map(|text| ContentItem::InputText { text })
        .collect();

    Some(ResponseItem::Message {
        id: None,
        role: role.to_string(),
        content,
        phase: None,
    })
}

pub(crate) fn build_settings_update_items(
    previous: Option<&TurnContextItem>,
    previous_turn_settings: Option<&PreviousTurnSettings>,
    next: &TurnContext,
    shell: &Shell,
) -> Vec<ResponseItem> {
    let contextual_user_message = build_environment_update_item(previous, next, shell);
    let developer_update_sections = [build_model_instructions_update_item(
        previous_turn_settings,
        next,
    )]
    .into_iter()
    .flatten()
    .collect();

    let mut items = Vec::with_capacity(2);
    if let Some(developer_message) = build_developer_update_item(developer_update_sections) {
        items.push(developer_message);
    }
    if let Some(contextual_user_message) = contextual_user_message {
        items.push(contextual_user_message);
    }
    items
}
