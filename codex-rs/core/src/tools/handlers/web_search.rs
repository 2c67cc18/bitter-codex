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
use codex_api::SearchQuery;
use codex_api::SearchRequest;
use codex_api::SearchSettings;
use codex_login::default_client::build_reqwest_client;
use codex_protocol::items::TurnItem;
use codex_protocol::items::WebSearchItem;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::models::WebSearchAction;
use codex_tools::JsonSchema;
use codex_tools::ResponsesApiNamespace;
use codex_tools::ResponsesApiNamespaceTool;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolName;
use codex_tools::ToolOutput;
use codex_tools::ToolSpec;
use codex_tools::default_namespace_description;
use codex_tools::parse_tool_input_schema_without_compaction;
use codex_utils_output_truncation::TruncationPolicy;
use codex_utils_output_truncation::approx_token_count;
use codex_utils_output_truncation::truncate_text;
use http::HeaderMap;
use reqwest::Url;

const WEB_NAMESPACE: &str = "web_run";
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

    fn supports_parallel_tool_calls(&self) -> bool {
        true
    }

    async fn handle(
        &self,
        invocation: ToolInvocation,
    ) -> Result<Box<dyn crate::tools::context::ToolOutput>, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            call_id,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "web search received unsupported payload".to_string(),
            ));
        };
        let commands = parse_commands(&arguments)?;
        let command_action = command_action(&commands);
        let started_item = web_search_item(&call_id, command_action.clone());
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
            model: turn.model_info.slug.clone(),
            reasoning: None,
            input: recent_input(&session, turn.as_ref()).await,
            commands: Some(commands),
            settings: Some(self.settings.clone()),
            max_output_tokens: Some(
                u64::try_from(turn.truncation_policy.token_budget()).unwrap_or(u64::MAX),
            ),
        };
        session
            .emit_turn_item_started(turn.as_ref(), &started_item)
            .await;
        let response = client
            .search(&request, HeaderMap::new())
            .await
            .map_err(|err| FunctionCallError::Fatal(err.to_string()))?;
        session
            .emit_turn_item_completed(turn.as_ref(), web_search_item(&call_id, command_action))
            .await;

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

fn command_action(commands: &SearchCommands) -> WebSearchAction {
    commands
        .search_query
        .as_deref()
        .and_then(query_action)
        .or_else(|| commands.image_query.as_deref().and_then(query_action))
        .or_else(|| {
            commands
                .open
                .as_deref()
                .and_then(|operations| operations.first())
                .and_then(|operation| {
                    literal_url(&operation.ref_id)
                        .map(|url| WebSearchAction::OpenPage { url: Some(url) })
                })
        })
        .or_else(|| {
            commands
                .find
                .as_deref()
                .and_then(|operations| operations.first())
                .map(|operation| WebSearchAction::FindInPage {
                    url: literal_url(&operation.ref_id),
                    pattern: Some(operation.pattern.clone()),
                })
        })
        .unwrap_or(WebSearchAction::Other)
}

fn query_action(queries: &[SearchQuery]) -> Option<WebSearchAction> {
    match queries {
        [] => None,
        [query] => Some(WebSearchAction::Search {
            query: Some(query.q.clone()),
            queries: None,
        }),
        queries => Some(WebSearchAction::Search {
            query: None,
            queries: Some(queries.iter().map(|query| query.q.clone()).collect()),
        }),
    }
}

fn literal_url(ref_id: &str) -> Option<String> {
    Url::parse(ref_id).is_ok().then(|| ref_id.to_string())
}

fn web_search_item(call_id: &str, action: WebSearchAction) -> TurnItem {
    TurnItem::WebSearch(WebSearchItem {
        id: call_id.to_string(),
        query: crate::web_search_action_detail(&action),
        action,
    })
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
    let schema = schemars::generate::SchemaSettings::draft2019_09()
        .with(|settings| {
            settings.inline_subschemas = true;
        })
        .into_generator()
        .into_root_schema_for::<SearchCommands>();
    let schema = serde_json::to_value(schema).expect("search commands schema should serialize");
    parse_tool_input_schema_without_compaction(&schema).expect("search command schema should parse")
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
    fn supports_parallel_tool_calls() {
        let handler = WebSearchHandler::new(SearchSettings::default());

        assert!(handler.supports_parallel_tool_calls());
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
            Some("Query the internet search engine for a given list of queries.".to_string())
        );
    }

    #[test]
    fn web_run_description_preserves_direct_standalone_guidance() {
        assert_eq!(
            web_run_description(),
            "Tool for accessing the internet. Use search_query, image_query, open, click, find, screenshot, finance, weather, sports, time, and response_length to retrieve current information. Search settings use direct standalone web access only."
        );
    }

    #[test]
    fn schema_preserves_sports_league_enum() {
        let schema = commands_schema();
        let sports = schema
            .properties
            .as_ref()
            .expect("schema properties")
            .get("sports")
            .expect("sports schema");
        let sports_item = sports.items.as_deref().expect("sports item schema");
        let league = sports_item
            .properties
            .as_ref()
            .expect("sports properties")
            .get("league")
            .expect("league schema");

        assert_eq!(
            league.enum_values,
            Some(vec![
                serde_json::json!("nba"),
                serde_json::json!("wnba"),
                serde_json::json!("nfl"),
                serde_json::json!("nhl"),
                serde_json::json!("mlb"),
                serde_json::json!("epl"),
                serde_json::json!("ncaamb"),
                serde_json::json!("ncaawb"),
                serde_json::json!("ipl"),
            ])
        );
    }

    #[test]
    fn command_action_reports_queries_and_navigation_detail() {
        let cases = [
            (
                r#"{"image_query":[{"q":"waterfalls"},{"q":"mountains"}]}"#,
                WebSearchAction::Search {
                    query: None,
                    queries: Some(vec!["waterfalls".to_string(), "mountains".to_string()]),
                },
            ),
            (
                r#"{"open":[{"ref_id":"https://example.com/docs"}]}"#,
                WebSearchAction::OpenPage {
                    url: Some("https://example.com/docs".to_string()),
                },
            ),
            (
                r#"{"find":[{"ref_id":"https://example.com/docs","pattern":"install"}]}"#,
                WebSearchAction::FindInPage {
                    url: Some("https://example.com/docs".to_string()),
                    pattern: Some("install".to_string()),
                },
            ),
            (
                r#"{"find":[{"ref_id":"turn0search0","pattern":"install"}]}"#,
                WebSearchAction::FindInPage {
                    url: None,
                    pattern: Some("install".to_string()),
                },
            ),
            (
                r#"{"open":[{"ref_id":"turn0search0"}]}"#,
                WebSearchAction::Other,
            ),
        ];

        for (arguments, expected) in cases {
            let commands: SearchCommands =
                serde_json::from_str(arguments).expect("valid search command arguments");
            assert_eq!(command_action(&commands), expected);
        }
    }

    #[test]
    fn web_search_item_includes_readable_query_detail() {
        let item = web_search_item(
            "call-1",
            WebSearchAction::Search {
                query: Some("standalone web search".to_string()),
                queries: None,
            },
        );

        let TurnItem::WebSearch(item) = item else {
            panic!("expected web search item");
        };

        assert_eq!(
            item,
            WebSearchItem {
                id: "call-1".to_string(),
                query: "standalone web search".to_string(),
                action: WebSearchAction::Search {
                    query: Some("standalone web search".to_string()),
                    queries: None,
                },
            }
        );
    }

    #[test]
    fn search_request_requires_model() {
        assert_eq!(
            serde_json::to_value(SearchRequest {
                id: "search-session".to_string(),
                model: "mock-model".to_string(),
                reasoning: None,
                input: None,
                commands: None,
                settings: None,
                max_output_tokens: None,
            })
            .expect("serialize request"),
            serde_json::json!({
                "id": "search-session",
                "model": "mock-model",
            })
        );
    }
}
