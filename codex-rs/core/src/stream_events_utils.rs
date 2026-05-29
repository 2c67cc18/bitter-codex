use std::pin::Pin;
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use codex_protocol::items::TurnItem;
use tokio_util::sync::CancellationToken;

use crate::context::ContextualUserFragment;
use crate::context::ImageGenerationInstructions;
use crate::function_tool::FunctionCallError;
use crate::parse_turn_item;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tools::parallel::ToolCallRuntime;
use crate::tools::router::ToolRouter;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::models::ResponseItem;
use codex_utils_absolute_path::AbsolutePathBuf;
use futures::Future;
use tracing::debug;
use tracing::instrument;

const GENERATED_IMAGE_ARTIFACTS_DIR: &str = "generated_images";

pub(crate) fn image_generation_artifact_path(
    codex_home: &AbsolutePathBuf,
    session_id: &str,
    call_id: &str,
) -> AbsolutePathBuf {
    let sanitize = |value: &str| {
        let mut sanitized: String = value
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                    ch
                } else {
                    '_'
                }
            })
            .collect();
        if sanitized.is_empty() {
            sanitized = "generated_image".to_string();
        }
        sanitized
    };

    codex_home
        .join(GENERATED_IMAGE_ARTIFACTS_DIR)
        .join(sanitize(session_id))
        .join(format!("{}.png", sanitize(call_id)))
}

fn strip_hidden_assistant_markup(text: &str, _plan_mode: bool) -> String {
    text.to_string()
}

pub(crate) fn raw_assistant_output_text_from_item(item: &ResponseItem) -> Option<String> {
    if let ResponseItem::Message { role, content, .. } = item
        && role == "assistant"
    {
        let combined = content
            .iter()
            .filter_map(|ci| match ci {
                codex_protocol::models::ContentItem::OutputText { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        return Some(combined);
    }
    None
}

async fn save_image_generation_result(
    codex_home: &AbsolutePathBuf,
    session_id: &str,
    call_id: &str,
    result: &str,
) -> Result<AbsolutePathBuf> {
    let bytes = BASE64_STANDARD
        .decode(result.trim().as_bytes())
        .map_err(|err| {
            CodexErr::InvalidRequest(format!("invalid image generation payload: {err}"))
        })?;
    let path = image_generation_artifact_path(codex_home, session_id, call_id);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&path, bytes).await?;
    Ok(path)
}

pub(crate) async fn record_completed_response_item(
    sess: &Session,
    turn_context: &TurnContext,
    item: &ResponseItem,
) {
    record_completed_response_item_with_finalized_facts(sess, turn_context, item, None).await;
}

pub(crate) async fn record_completed_response_item_with_finalized_facts(
    sess: &Session,
    turn_context: &TurnContext,
    item: &ResponseItem,
    _finalized_facts: Option<&FinalizedTurnItemFacts>,
) {
    sess.record_conversation_items(turn_context, std::slice::from_ref(item))
        .await;
}

pub(crate) type InFlightFuture<'f> =
    Pin<Box<dyn Future<Output = Result<ResponseInputItem>> + Send + 'f>>;

#[derive(Default)]
pub(crate) struct OutputItemResult {
    pub last_agent_message: Option<String>,
    pub needs_follow_up: bool,
    pub tool_future: Option<InFlightFuture<'static>>,
}

pub(crate) struct HandleOutputCtx {
    pub sess: Arc<Session>,
    pub turn_context: Arc<TurnContext>,
    pub tool_runtime: ToolCallRuntime,
    pub cancellation_token: CancellationToken,
}

pub(crate) struct FinalizedTurnItem {
    pub(crate) turn_item: TurnItem,
    pub(crate) facts: FinalizedTurnItemFacts,
}

#[derive(Clone, Default)]
pub(crate) struct FinalizedTurnItemFacts {
    pub(crate) last_agent_message: Option<String>,
}

pub(crate) async fn finalize_non_tool_response_item(
    sess: &Session,
    turn_context: &TurnContext,
    item: &ResponseItem,
    plan_mode: bool,
) -> Option<FinalizedTurnItem> {
    let turn_item = handle_non_tool_response_item(sess, turn_context, item, plan_mode).await?;
    let last_agent_message = match &turn_item {
        TurnItem::AgentMessage(agent_message) => {
            let combined = agent_message
                .content
                .iter()
                .map(|entry| match entry {
                    codex_protocol::items::AgentMessageContent::Text { text } => text.as_str(),
                })
                .collect::<String>();
            let last_agent_message = if combined.trim().is_empty() {
                None
            } else {
                Some(combined)
            };
            last_agent_message
        }
        _ => None,
    };
    Some(FinalizedTurnItem {
        turn_item,
        facts: FinalizedTurnItemFacts { last_agent_message },
    })
}

#[instrument(level = "trace", skip_all)]
pub(crate) async fn handle_output_item_done(
    ctx: &mut HandleOutputCtx,
    item: ResponseItem,
    previously_active_item: Option<TurnItem>,
) -> Result<OutputItemResult> {
    let mut output = OutputItemResult::default();
    let plan_mode = false;

    match ToolRouter::build_tool_call(item.clone()) {
        Ok(Some(call)) => {
            let payload_preview = call.payload.log_payload().into_owned();
            tracing::info!(
                thread_id = %ctx.sess.conversation_id,
                "ToolCall: {} {}",
                call.tool_name,
                payload_preview
            );

            record_completed_response_item(ctx.sess.as_ref(), ctx.turn_context.as_ref(), &item)
                .await;

            let cancellation_token = ctx.cancellation_token.child_token();
            let tool_future: InFlightFuture<'static> = Box::pin(
                ctx.tool_runtime
                    .clone()
                    .handle_tool_call(call, cancellation_token),
            );

            output.needs_follow_up = true;
            output.tool_future = Some(tool_future);
        }

        Ok(None) => {
            let finalized_turn_item = finalize_non_tool_response_item(
                ctx.sess.as_ref(),
                ctx.turn_context.as_ref(),
                &item,
                plan_mode,
            )
            .await;
            let finalized_facts = finalized_turn_item
                .as_ref()
                .map(|finalized| finalized.facts.clone());
            if let Some(finalized_turn_item) = finalized_turn_item {
                if previously_active_item.is_none() {
                    let mut started_item = finalized_turn_item.turn_item.clone();
                    if let TurnItem::ImageGeneration(item) = &mut started_item {
                        item.status = "in_progress".to_string();
                        item.revised_prompt = None;
                        item.result.clear();
                        item.saved_path = None;
                    }
                    ctx.sess
                        .emit_turn_item_started(&ctx.turn_context, &started_item)
                        .await;
                }

                ctx.sess
                    .emit_turn_item_completed(&ctx.turn_context, finalized_turn_item.turn_item)
                    .await;
            }
            record_completed_response_item_with_finalized_facts(
                ctx.sess.as_ref(),
                ctx.turn_context.as_ref(),
                &item,
                finalized_facts.as_ref(),
            )
            .await;

            output.last_agent_message = finalized_facts.and_then(|facts| facts.last_agent_message);
        }

        Err(FunctionCallError::RespondToModel(message)) => {
            let response = ResponseInputItem::FunctionCallOutput {
                call_id: String::new(),
                output: FunctionCallOutputPayload {
                    body: FunctionCallOutputBody::Text(message),
                    ..Default::default()
                },
            };
            record_completed_response_item(ctx.sess.as_ref(), ctx.turn_context.as_ref(), &item)
                .await;
            if let Some(response_item) = response_input_to_response_item(&response) {
                ctx.sess
                    .record_conversation_items(
                        &ctx.turn_context,
                        std::slice::from_ref(&response_item),
                    )
                    .await;
            }

            output.needs_follow_up = true;
        }

        Err(FunctionCallError::Fatal(message)) => {
            return Err(CodexErr::Fatal(message));
        }
    }

    Ok(output)
}

pub(crate) async fn handle_non_tool_response_item(
    sess: &Session,
    turn_context: &TurnContext,
    item: &ResponseItem,
    plan_mode: bool,
) -> Option<TurnItem> {
    debug!(?item, "Output item");

    match item {
        ResponseItem::Message { .. }
        | ResponseItem::Reasoning { .. }
        | ResponseItem::WebSearchCall { .. }
        | ResponseItem::ImageGenerationCall { .. } => {
            let mut turn_item = parse_turn_item(item)?;
            if let TurnItem::AgentMessage(agent_message) = &mut turn_item {
                let combined = agent_message
                    .content
                    .iter()
                    .map(|entry| match entry {
                        codex_protocol::items::AgentMessageContent::Text { text } => text.as_str(),
                    })
                    .collect::<String>();
                let stripped = strip_hidden_assistant_markup(&combined, plan_mode);
                agent_message.content =
                    vec![codex_protocol::items::AgentMessageContent::Text { text: stripped }];
            }
            if let TurnItem::ImageGeneration(image_item) = &mut turn_item {
                let session_id = sess.conversation_id.to_string();
                match save_image_generation_result(
                    &turn_context.config.codex_home,
                    &session_id,
                    &image_item.id,
                    &image_item.result,
                )
                .await
                {
                    Ok(path) => {
                        image_item.saved_path = Some(path);
                        let image_output_path = image_generation_artifact_path(
                            &turn_context.config.codex_home,
                            &session_id,
                            "<image_id>",
                        );
                        let image_output_dir = image_output_path
                            .parent()
                            .unwrap_or_else(|| turn_context.config.codex_home.clone());
                        let message: ResponseItem =
                            ContextualUserFragment::into(ImageGenerationInstructions::new(
                                image_output_dir.display(),
                                image_output_path.display(),
                            ));
                        sess.record_conversation_items(turn_context, &[message])
                            .await;
                    }
                    Err(err) => {
                        let output_path = image_generation_artifact_path(
                            &turn_context.config.codex_home,
                            &session_id,
                            &image_item.id,
                        );
                        let output_dir = output_path
                            .parent()
                            .unwrap_or_else(|| turn_context.config.codex_home.clone());
                        tracing::warn!(
                            call_id = %image_item.id,
                            output_dir = %output_dir.display(),
                            "failed to save generated image: {err}"
                        );
                    }
                }
            }
            Some(turn_item)
        }
        ResponseItem::FunctionCallOutput { .. } | ResponseItem::CustomToolCallOutput { .. } => {
            debug!("unexpected tool output from stream");
            None
        }
        _ => None,
    }
}

pub(crate) fn last_assistant_message_from_item(
    item: &ResponseItem,
    plan_mode: bool,
) -> Option<String> {
    if let Some(combined) = raw_assistant_output_text_from_item(item) {
        if combined.is_empty() {
            return None;
        }
        let stripped = strip_hidden_assistant_markup(&combined, plan_mode);
        if stripped.trim().is_empty() {
            return None;
        }
        return Some(stripped);
    }
    None
}

pub(crate) fn response_input_to_response_item(input: &ResponseInputItem) -> Option<ResponseItem> {
    match input {
        ResponseInputItem::FunctionCallOutput { call_id, output } => {
            Some(ResponseItem::FunctionCallOutput {
                call_id: call_id.clone(),
                output: output.clone(),
            })
        }
        _ => None,
    }
}
