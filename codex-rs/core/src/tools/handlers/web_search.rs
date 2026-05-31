use crate::event_mapping::parse_turn_item;
use crate::function_tool::FunctionCallError;
use crate::session::turn_context::TurnContext;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::registry::CoreToolRuntime;
use crate::tools::registry::ToolExecutor;
use codex_api::ReqwestTransport;
use codex_api::SearchClient;
use codex_api::SearchCommands;
use codex_api::SearchInput;
use codex_api::SearchRequest;
use codex_api::SearchSettings;
use codex_login::default_client::build_reqwest_client;
use codex_protocol::items::TurnItem;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::models::ResponseItem;
use codex_tools::JsonSchema;
use codex_tools::ResponsesApiNamespace;
use codex_tools::ResponsesApiNamespaceTool;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolName;
use codex_tools::ToolOutput;
use codex_tools::ToolSpec;
use codex_tools::default_namespace_description;
use codex_utils_output_truncation::TruncationPolicy;
use codex_utils_output_truncation::approx_token_count;
use codex_utils_output_truncation::truncate_text;
use http::HeaderMap;
use serde_json::json;
use std::collections::BTreeMap;

const WEB_NAMESPACE: &str = "web";
const RUN_TOOL_NAME: &str = "run";
const ASSISTANT_CONTEXT_TOKEN_LIMIT: usize = 1_000;

pub(crate) struct WebSearchHandler {
    settings: SearchSettings,
}

impl WebSearchHandler {
    pub(crate) fn new(settings: SearchSettings) -> Self {
        Self { settings }
    }
}

#[async_trait::async_trait]
impl ToolExecutor<ToolInvocation> for WebSearchHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::new(Some(WEB_NAMESPACE.to_string()), RUN_TOOL_NAME.to_string())
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec::Namespace(ResponsesApiNamespace {
            name: WEB_NAMESPACE.to_string(),
            description: default_namespace_description(WEB_NAMESPACE),
            tools: vec![ResponsesApiNamespaceTool::Function(ResponsesApiTool {
                name: RUN_TOOL_NAME.to_string(),
                description: web_run_description(),
                strict: false,
                parameters: commands_schema(),
                output_schema: None,
                defer_loading: None,
            })],
        })
    }

    async fn handle(
        &self,
        invocation: ToolInvocation,
    ) -> Result<Box<dyn crate::tools::context::ToolOutput>, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "web search received unsupported payload".to_string(),
            ));
        };
        let commands = parse_commands(&arguments)?;
        let provider = turn
            .provider
            .api_provider()
            .await
            .map_err(|err| FunctionCallError::Fatal(err.to_string()))?;
        let auth = turn
            .provider
            .api_auth()
            .await
            .map_err(|err| FunctionCallError::Fatal(err.to_string()))?;
        let client = SearchClient::new(
            ReqwestTransport::new(build_reqwest_client()),
            provider,
            auth,
        );
        let request = SearchRequest {
            id: session.session_id().to_string(),
            model: None,
            reasoning: None,
            input: recent_input(&session, turn.as_ref()).await,
            commands: Some(commands),
            settings: Some(self.settings.clone()),
            max_output_tokens: Some(
                u64::try_from(turn.truncation_policy.token_budget()).unwrap_or(u64::MAX),
            ),
        };
        let response = client
            .search(&request, HeaderMap::new())
            .await
            .map_err(|err| FunctionCallError::Fatal(err.to_string()))?;

        Ok(boxed_tool_output(EncryptedSearchOutput {
            encrypted_output: response.encrypted_output,
        }))
    }
}

impl CoreToolRuntime for WebSearchHandler {}

fn parse_commands(arguments: &str) -> Result<SearchCommands, FunctionCallError> {
    if arguments.trim().is_empty() {
        return Ok(SearchCommands::default());
    }
    serde_json::from_str(arguments)
        .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))
}

async fn recent_input(
    session: &crate::session::session::Session,
    _turn_context: &TurnContext,
) -> Option<SearchInput> {
    let items = session.clone_history().await.raw_items().to_vec();
    let mut messages = Vec::new();
    for item in items {
        push_visible_message(&mut messages, &item);
    }
    retain_tail_from_last_n_user_messages(&mut messages, 2);
    truncate_assistant_output_text_to_token_budget(&mut messages, ASSISTANT_CONTEXT_TOKEN_LIMIT);
    (!messages.is_empty()).then_some(SearchInput::Items(messages))
}

fn push_visible_message(messages: &mut Vec<ResponseItem>, item: &ResponseItem) {
    match item {
        ResponseItem::Message { role, .. } if role == "assistant" => {
            messages.push(item.clone());
        }
        ResponseItem::Message {
            id,
            role,
            content,
            phase,
        } if role == "user" && matches!(parse_turn_item(item), Some(TurnItem::UserMessage(_))) => {
            let content = content
                .iter()
                .filter(|item| matches!(item, ContentItem::InputText { .. }))
                .cloned()
                .collect::<Vec<_>>();
            if !content.is_empty() {
                messages.push(ResponseItem::Message {
                    id: id.clone(),
                    role: role.clone(),
                    content,
                    phase: phase.clone(),
                });
            }
        }
        _ => {}
    }
}

fn retain_tail_from_last_n_user_messages(items: &mut Vec<ResponseItem>, user_message_count: usize) {
    if user_message_count == 0 {
        items.clear();
        return;
    }
    let Some(latest_user_idx) = items.iter().rposition(is_user_message) else {
        items.clear();
        return;
    };
    items.truncate(latest_user_idx + 1);
    let earliest_retained_user_idx = items
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, item)| is_user_message(item))
        .take(user_message_count)
        .last()
        .map(|(idx, _)| idx)
        .unwrap_or(latest_user_idx);
    items.drain(..earliest_retained_user_idx);
}

fn is_user_message(item: &ResponseItem) -> bool {
    matches!(item, ResponseItem::Message { role, .. } if role == "user")
}

fn truncate_assistant_output_text_to_token_budget(
    items: &mut Vec<ResponseItem>,
    max_tokens: usize,
) {
    let mut remaining_budget = max_tokens;
    items.retain_mut(|item| {
        let ResponseItem::Message { role, content, .. } = item else {
            return true;
        };
        if role != "assistant" {
            return true;
        }
        content.retain_mut(|content_item| {
            let ContentItem::OutputText { text } = content_item else {
                return true;
            };
            if remaining_budget == 0 {
                return false;
            }
            let token_count = approx_token_count(text);
            if token_count <= remaining_budget {
                remaining_budget = remaining_budget.saturating_sub(token_count);
                return true;
            }
            *text = truncate_text(text, TruncationPolicy::Tokens(remaining_budget));
            remaining_budget = 0;
            true
        });
        !content.is_empty()
    });
}

struct EncryptedSearchOutput {
    encrypted_output: String,
}

impl ToolOutput for EncryptedSearchOutput {
    fn log_preview(&self) -> String {
        "[encrypted standalone web search output]".to_string()
    }

    fn success_for_logging(&self) -> bool {
        true
    }

    fn to_response_item(&self, call_id: &str, _payload: &ToolPayload) -> ResponseInputItem {
        ResponseInputItem::FunctionCallOutput {
            call_id: call_id.to_string(),
            output: FunctionCallOutputPayload::from_content_items(vec![
                FunctionCallOutputContentItem::EncryptedContent {
                    encrypted_content: self.encrypted_output.clone(),
                },
            ]),
        }
    }
}

fn web_run_description() -> String {
    "Tool for accessing the internet. Use search_query, image_query, open, click, find, screenshot, finance, weather, sports, time, and response_length to retrieve current information. Search settings use direct standalone web access only.".to_string()
}

fn commands_schema() -> JsonSchema {
    let search_query = JsonSchema::object(
        BTreeMap::from([
            (
                "q".to_string(),
                JsonSchema::string(Some("Search query".to_string())),
            ),
            (
                "recency".to_string(),
                JsonSchema::integer(Some("Optional recency in days".to_string())),
            ),
            (
                "domains".to_string(),
                JsonSchema::array(
                    JsonSchema::string(None),
                    Some("Optional domain filters".to_string()),
                ),
            ),
        ]),
        Some(vec!["q".to_string()]),
        Some(false.into()),
    );
    let ref_id = JsonSchema::string(Some(
        "Reference id or URL returned by a previous web operation".to_string(),
    ));
    JsonSchema::object(
        BTreeMap::from([
            (
                "search_query".to_string(),
                JsonSchema::array(
                    search_query.clone(),
                    Some("Search the web for text results".to_string()),
                ),
            ),
            (
                "image_query".to_string(),
                JsonSchema::array(
                    search_query,
                    Some("Search the web for image results".to_string()),
                ),
            ),
            (
                "open".to_string(),
                JsonSchema::array(
                    JsonSchema::object(
                        BTreeMap::from([
                            ("ref_id".to_string(), ref_id.clone()),
                            (
                                "lineno".to_string(),
                                JsonSchema::integer(Some("Optional line number".to_string())),
                            ),
                        ]),
                        Some(vec!["ref_id".to_string()]),
                        Some(false.into()),
                    ),
                    Some("Open a search result or URL".to_string()),
                ),
            ),
            (
                "click".to_string(),
                JsonSchema::array(
                    JsonSchema::object(
                        BTreeMap::from([
                            ("ref_id".to_string(), ref_id.clone()),
                            (
                                "id".to_string(),
                                JsonSchema::integer(Some("Link id to click".to_string())),
                            ),
                        ]),
                        Some(vec!["ref_id".to_string(), "id".to_string()]),
                        Some(false.into()),
                    ),
                    Some("Click a link from an opened page".to_string()),
                ),
            ),
            (
                "find".to_string(),
                JsonSchema::array(
                    JsonSchema::object(
                        BTreeMap::from([
                            ("ref_id".to_string(), ref_id.clone()),
                            (
                                "pattern".to_string(),
                                JsonSchema::string(Some("Text to find".to_string())),
                            ),
                        ]),
                        Some(vec!["ref_id".to_string(), "pattern".to_string()]),
                        Some(false.into()),
                    ),
                    Some("Find text in an opened page".to_string()),
                ),
            ),
            (
                "screenshot".to_string(),
                JsonSchema::array(
                    JsonSchema::object(
                        BTreeMap::from([
                            ("ref_id".to_string(), ref_id),
                            (
                                "pageno".to_string(),
                                JsonSchema::integer(Some("Zero-based PDF page number".to_string())),
                            ),
                        ]),
                        Some(vec!["ref_id".to_string(), "pageno".to_string()]),
                        Some(false.into()),
                    ),
                    Some("Capture a PDF page screenshot".to_string()),
                ),
            ),
            (
                "finance".to_string(),
                JsonSchema::array(
                    JsonSchema::object(
                        BTreeMap::from([
                            (
                                "ticker".to_string(),
                                JsonSchema::string(Some("Ticker symbol".to_string())),
                            ),
                            (
                                "type".to_string(),
                                JsonSchema::string_enum(
                                    vec![
                                        json!("equity"),
                                        json!("fund"),
                                        json!("crypto"),
                                        json!("index"),
                                    ],
                                    Some("Asset type".to_string()),
                                ),
                            ),
                            (
                                "market".to_string(),
                                JsonSchema::string(Some("Market code".to_string())),
                            ),
                        ]),
                        Some(vec!["ticker".to_string(), "type".to_string()]),
                        Some(false.into()),
                    ),
                    Some("Look up financial prices".to_string()),
                ),
            ),
            (
                "weather".to_string(),
                JsonSchema::array(
                    JsonSchema::object(
                        BTreeMap::from([
                            (
                                "location".to_string(),
                                JsonSchema::string(Some("Location".to_string())),
                            ),
                            (
                                "start".to_string(),
                                JsonSchema::string(Some("Start date YYYY-MM-DD".to_string())),
                            ),
                            (
                                "duration".to_string(),
                                JsonSchema::integer(Some("Number of days".to_string())),
                            ),
                        ]),
                        Some(vec!["location".to_string()]),
                        Some(false.into()),
                    ),
                    Some("Look up weather".to_string()),
                ),
            ),
            (
                "sports".to_string(),
                JsonSchema::array(
                    JsonSchema::object(
                        BTreeMap::from([
                            (
                                "tool".to_string(),
                                JsonSchema::string_enum(
                                    vec![json!("sports")],
                                    Some("Tool discriminator".to_string()),
                                ),
                            ),
                            (
                                "fn".to_string(),
                                JsonSchema::string_enum(
                                    vec![json!("schedule"), json!("standings")],
                                    Some("Sports operation".to_string()),
                                ),
                            ),
                            (
                                "league".to_string(),
                                JsonSchema::string(Some("League".to_string())),
                            ),
                            (
                                "team".to_string(),
                                JsonSchema::string(Some("Team".to_string())),
                            ),
                            (
                                "opponent".to_string(),
                                JsonSchema::string(Some("Opponent".to_string())),
                            ),
                            (
                                "date_from".to_string(),
                                JsonSchema::string(Some("Start date YYYY-MM-DD".to_string())),
                            ),
                            (
                                "date_to".to_string(),
                                JsonSchema::string(Some("End date YYYY-MM-DD".to_string())),
                            ),
                            (
                                "num_games".to_string(),
                                JsonSchema::integer(Some("Number of games".to_string())),
                            ),
                            (
                                "locale".to_string(),
                                JsonSchema::string(Some("Locale".to_string())),
                            ),
                        ]),
                        Some(vec!["fn".to_string(), "league".to_string()]),
                        Some(false.into()),
                    ),
                    Some("Look up sports schedules or standings".to_string()),
                ),
            ),
            (
                "time".to_string(),
                JsonSchema::array(
                    JsonSchema::object(
                        BTreeMap::from([(
                            "utc_offset".to_string(),
                            JsonSchema::string(Some("UTC offset such as +03:00".to_string())),
                        )]),
                        Some(vec!["utc_offset".to_string()]),
                        Some(false.into()),
                    ),
                    Some("Get local time for UTC offsets".to_string()),
                ),
            ),
            (
                "response_length".to_string(),
                JsonSchema::string_enum(
                    vec![json!("short"), json!("medium"), json!("long")],
                    Some("Response length".to_string()),
                ),
            ),
        ]),
        None,
        Some(false.into()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_api::SearchQuery;
    use pretty_assertions::assert_eq;

    fn message(role: &str, text: &str) -> ResponseItem {
        ResponseItem::Message {
            id: None,
            role: role.to_string(),
            content: vec![if role == "assistant" {
                ContentItem::OutputText {
                    text: text.to_string(),
                }
            } else {
                ContentItem::InputText {
                    text: text.to_string(),
                }
            }],
            phase: None,
        }
    }

    #[test]
    fn parses_upstream_search_command_shape() {
        let commands = parse_commands(
            r#"{
                "search_query": [{"q": "OpenAI news", "recency": 7}],
                "response_length": "short"
            }"#,
        )
        .expect("commands should parse");

        assert_eq!(
            commands,
            SearchCommands {
                search_query: Some(vec![SearchQuery {
                    q: "OpenAI news".to_string(),
                    recency: Some(7),
                    domains: None,
                }]),
                response_length: Some(codex_api::SearchResponseLength::Short),
                ..Default::default()
            }
        );
    }

    #[test]
    fn response_history_keeps_recent_user_tail() {
        let mut items = vec![
            message("user", "old user"),
            message("assistant", "old assistant"),
            message("user", "previous user"),
            message("assistant", "previous assistant"),
            message("user", "current user"),
            message("assistant", "later assistant"),
        ];

        retain_tail_from_last_n_user_messages(&mut items, 2);

        assert_eq!(
            items,
            vec![
                message("user", "previous user"),
                message("assistant", "previous assistant"),
                message("user", "current user"),
            ]
        );
    }

    #[test]
    fn emits_encrypted_function_output() {
        let output = EncryptedSearchOutput {
            encrypted_output: "ciphertext".to_string(),
        };

        assert_eq!(
            output.to_response_item(
                "call-1",
                &ToolPayload::Function {
                    arguments: "{}".to_string(),
                },
            ),
            ResponseInputItem::FunctionCallOutput {
                call_id: "call-1".to_string(),
                output: FunctionCallOutputPayload::from_content_items(vec![
                    FunctionCallOutputContentItem::EncryptedContent {
                        encrypted_content: "ciphertext".to_string(),
                    },
                ]),
            }
        );
    }

    #[test]
    fn schema_preserves_command_field_guidance() {
        let schema = commands_schema();
        let search_query = schema
            .properties
            .as_ref()
            .expect("schema properties")
            .get("search_query")
            .expect("search_query schema");

        assert_eq!(
            search_query.description,
            Some("Search the web for text results".to_string())
        );
    }
}
