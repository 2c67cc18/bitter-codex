use crate::mcp::CallToolResult;
use crate::memory_citation::MemoryCitation;
use crate::models::ImageDetail;
use crate::models::MessagePhase;
use crate::models::WebSearchAction;
use crate::protocol::AgentMessageEvent;
use crate::protocol::AgentReasoningEvent;
use crate::protocol::AgentReasoningRawContentEvent;
use crate::protocol::ContextCompactedEvent;
use crate::protocol::EventMsg;
use crate::protocol::FileChange;
use crate::protocol::ImageGenerationEndEvent;
use crate::protocol::McpInvocation;
use crate::protocol::McpToolCallBeginEvent;
use crate::protocol::McpToolCallEndEvent;
use crate::protocol::PatchApplyBeginEvent;
use crate::protocol::PatchApplyEndEvent;
use crate::protocol::PatchApplyStatus;
use crate::protocol::UserMessageEvent;
use crate::protocol::ViewImageToolCallEvent;
use crate::protocol::WebSearchEndEvent;
use crate::user_input::ByteRange;
use crate::user_input::TextElement;
use crate::user_input::UserInput;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum TurnItem {
    UserMessage(UserMessageItem),
    AgentMessage(AgentMessageItem),
    Plan(PlanItem),
    Reasoning(ReasoningItem),
    WebSearch(WebSearchItem),
    ImageView(ImageViewItem),
    ImageGeneration(ImageGenerationItem),
    FileChange(FileChangeItem),
    McpToolCall(McpToolCallItem),
    ContextCompaction(ContextCompactionItem),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UserMessageItem {
    pub id: String,
    pub content: Vec<UserInput>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum AgentMessageContent {
    Text { text: String },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
/// Assistant-authored message payload used in turn-item streams.
///
/// `phase` is optional because not all providers/models emit it. Consumers
/// should use it when present, but retain legacy completion semantics when it
/// is `None`.
pub struct AgentMessageItem {
    pub id: String,
    pub content: Vec<AgentMessageContent>,
    /// Optional phase metadata carried through from `ResponseItem::Message`.
    ///
    /// This is currently used by TUI rendering to distinguish mid-turn
    /// commentary from a final answer and avoid status-indicator jitter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<MessagePhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_citation: Option<MemoryCitation>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlanItem {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReasoningItem {
    pub id: String,
    pub summary_text: Vec<String>,
    #[serde(default)]
    pub raw_content: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct WebSearchItem {
    pub id: String,
    pub query: String,
    pub action: WebSearchAction,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ImageViewItem {
    pub id: String,
    pub path: AbsolutePathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ImageGenerationItem {
    pub id: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revised_prompt: Option<String>,
    pub result: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_path: Option<AbsolutePathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct FileChangeItem {
    pub id: String,
    pub changes: HashMap<PathBuf, FileChange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<PatchApplyStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_approved: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpToolCallItem {
    pub id: String,
    pub server: String,
    pub tool: String,
    pub arguments: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_app_resource_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    pub status: McpToolCallStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<CallToolResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<McpToolCallError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<Duration>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum McpToolCallStatus {
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpToolCallError {
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContextCompactionItem {
    pub id: String,
}

impl ContextCompactionItem {
    pub fn new() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
        }
    }

    pub fn as_legacy_event(&self) -> EventMsg {
        EventMsg::ContextCompacted(ContextCompactedEvent {})
    }
}

impl Default for ContextCompactionItem {
    fn default() -> Self {
        Self::new()
    }
}

impl UserMessageItem {
    pub fn new(content: &[UserInput]) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            content: content.to_vec(),
        }
    }

    pub fn as_legacy_event(&self) -> EventMsg {
        // Legacy user-message events flatten only text inputs into `message` and
        // rebase text element ranges onto that concatenated text.
        EventMsg::UserMessage(UserMessageEvent {
            message: self.message(),
            images: Some(self.image_urls()),
            image_details: self.image_details(),
            local_images: self.local_image_paths(),
            local_image_details: self.local_image_details(),
            text_elements: self.text_elements(),
        })
    }

    pub fn message(&self) -> String {
        self.content
            .iter()
            .map(|c| match c {
                UserInput::Text { text, .. } => text.clone(),
                _ => String::new(),
            })
            .collect::<Vec<String>>()
            .join("")
    }

    pub fn text_elements(&self) -> Vec<TextElement> {
        let mut out = Vec::new();
        let mut offset = 0usize;
        for input in &self.content {
            if let UserInput::Text {
                text,
                text_elements,
            } = input
            {
                // Text element ranges are relative to each text chunk; offset them so they align
                // with the concatenated message returned by `message()`.
                for elem in text_elements {
                    let byte_range = ByteRange {
                        start: offset + elem.byte_range.start,
                        end: offset + elem.byte_range.end,
                    };
                    out.push(TextElement::new(
                        byte_range,
                        elem.placeholder(text).map(str::to_string),
                    ));
                }
                offset += text.len();
            }
        }
        out
    }

    pub fn image_urls(&self) -> Vec<String> {
        self.content
            .iter()
            .filter_map(|c| match c {
                UserInput::Image { image_url, .. } => Some(image_url.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn image_details(&self) -> Vec<Option<ImageDetail>> {
        trim_trailing_default_image_details(
            self.content
                .iter()
                .filter_map(|c| match c {
                    UserInput::Image { detail, .. } => Some(*detail),
                    _ => None,
                })
                .collect(),
        )
    }

    pub fn local_image_paths(&self) -> Vec<std::path::PathBuf> {
        self.content
            .iter()
            .filter_map(|c| match c {
                UserInput::LocalImage { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn local_image_details(&self) -> Vec<Option<ImageDetail>> {
        trim_trailing_default_image_details(
            self.content
                .iter()
                .filter_map(|c| match c {
                    UserInput::LocalImage { detail, .. } => Some(*detail),
                    _ => None,
                })
                .collect(),
        )
    }
}

fn trim_trailing_default_image_details(
    mut details: Vec<Option<ImageDetail>>,
) -> Vec<Option<ImageDetail>> {
    while matches!(details.last(), Some(None)) {
        details.pop();
    }
    details
}

impl AgentMessageItem {
    pub fn new(content: &[AgentMessageContent]) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            content: content.to_vec(),
            phase: None,
            memory_citation: None,
        }
    }

    pub fn as_legacy_events(&self) -> Vec<EventMsg> {
        self.content
            .iter()
            .map(|c| match c {
                AgentMessageContent::Text { text } => EventMsg::AgentMessage(AgentMessageEvent {
                    message: text.clone(),
                    phase: self.phase.clone(),
                    memory_citation: self.memory_citation.clone(),
                }),
            })
            .collect()
    }
}

impl ReasoningItem {
    pub fn as_legacy_events(&self, show_raw_agent_reasoning: bool) -> Vec<EventMsg> {
        let mut events = Vec::new();
        for summary in &self.summary_text {
            events.push(EventMsg::AgentReasoning(AgentReasoningEvent {
                text: summary.clone(),
            }));
        }

        if show_raw_agent_reasoning {
            for entry in &self.raw_content {
                events.push(EventMsg::AgentReasoningRawContent(
                    AgentReasoningRawContentEvent {
                        text: entry.clone(),
                    },
                ));
            }
        }

        events
    }
}

impl WebSearchItem {
    pub fn as_legacy_event(&self) -> EventMsg {
        EventMsg::WebSearchEnd(WebSearchEndEvent {
            call_id: self.id.clone(),
            query: self.query.clone(),
            action: self.action.clone(),
        })
    }
}

impl ImageGenerationItem {
    pub fn as_legacy_event(&self) -> EventMsg {
        EventMsg::ImageGenerationEnd(ImageGenerationEndEvent {
            call_id: self.id.clone(),
            status: self.status.clone(),
            revised_prompt: self.revised_prompt.clone(),
            result: self.result.clone(),
            saved_path: self.saved_path.clone(),
        })
    }
}

impl FileChangeItem {
    pub fn as_legacy_begin_event(&self, turn_id: String) -> EventMsg {
        EventMsg::PatchApplyBegin(PatchApplyBeginEvent {
            call_id: self.id.clone(),
            turn_id,
            auto_approved: self.auto_approved.unwrap_or(false),
            changes: self.changes.clone(),
        })
    }

    pub fn as_legacy_end_event(&self, turn_id: String) -> Option<EventMsg> {
        let status = self.status.clone()?;
        Some(EventMsg::PatchApplyEnd(PatchApplyEndEvent {
            call_id: self.id.clone(),
            turn_id,
            stdout: self.stdout.clone().unwrap_or_default(),
            stderr: self.stderr.clone().unwrap_or_default(),
            success: status == PatchApplyStatus::Completed,
            changes: self.changes.clone(),
            status,
        }))
    }
}

impl McpToolCallItem {
    pub fn as_legacy_begin_event(&self) -> EventMsg {
        EventMsg::McpToolCallBegin(McpToolCallBeginEvent {
            call_id: self.id.clone(),
            invocation: McpInvocation {
                server: self.server.clone(),
                tool: self.tool.clone(),
                arguments: (!self.arguments.is_null()).then(|| self.arguments.clone()),
            },
            mcp_app_resource_uri: self.mcp_app_resource_uri.clone(),
            plugin_id: self.plugin_id.clone(),
        })
    }

    pub fn as_legacy_end_event(&self) -> Option<EventMsg> {
        let result = match (&self.result, &self.error) {
            (Some(result), _) => Ok(result.clone()),
            (None, Some(error)) => Err(error.message.clone()),
            (None, None) => return None,
        };

        Some(EventMsg::McpToolCallEnd(McpToolCallEndEvent {
            call_id: self.id.clone(),
            invocation: McpInvocation {
                server: self.server.clone(),
                tool: self.tool.clone(),
                arguments: (!self.arguments.is_null()).then(|| self.arguments.clone()),
            },
            mcp_app_resource_uri: self.mcp_app_resource_uri.clone(),
            plugin_id: self.plugin_id.clone(),
            duration: self.duration?,
            result,
        }))
    }
}

impl TurnItem {
    pub fn id(&self) -> String {
        match self {
            TurnItem::UserMessage(item) => item.id.clone(),
            TurnItem::AgentMessage(item) => item.id.clone(),
            TurnItem::Plan(item) => item.id.clone(),
            TurnItem::Reasoning(item) => item.id.clone(),
            TurnItem::WebSearch(item) => item.id.clone(),
            TurnItem::ImageView(item) => item.id.clone(),
            TurnItem::ImageGeneration(item) => item.id.clone(),
            TurnItem::FileChange(item) => item.id.clone(),
            TurnItem::McpToolCall(item) => item.id.clone(),
            TurnItem::ContextCompaction(item) => item.id.clone(),
        }
    }

    pub fn as_legacy_events(&self, show_raw_agent_reasoning: bool) -> Vec<EventMsg> {
        match self {
            TurnItem::UserMessage(item) => vec![item.as_legacy_event()],
            TurnItem::AgentMessage(item) => item.as_legacy_events(),
            TurnItem::Plan(_) => Vec::new(),
            TurnItem::WebSearch(item) => vec![item.as_legacy_event()],
            TurnItem::ImageView(item) => {
                vec![EventMsg::ViewImageToolCall(ViewImageToolCallEvent {
                    call_id: item.id.clone(),
                    path: item.path.clone(),
                })]
            }
            TurnItem::ImageGeneration(item) => vec![item.as_legacy_event()],
            TurnItem::FileChange(item) => item
                .as_legacy_end_event(String::new())
                .into_iter()
                .collect(),
            TurnItem::McpToolCall(item) => item.as_legacy_end_event().into_iter().collect(),
            TurnItem::Reasoning(item) => item.as_legacy_events(show_raw_agent_reasoning),
            TurnItem::ContextCompaction(item) => vec![item.as_legacy_event()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
}
