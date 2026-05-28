use codex_protocol::models::WebSearchAction;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum ThreadEvent {
    #[serde(rename = "thread.started")]
    ThreadStarted(ThreadStartedEvent),

    #[serde(rename = "turn.started")]
    TurnStarted(TurnStartedEvent),

    #[serde(rename = "turn.completed")]
    TurnCompleted(TurnCompletedEvent),

    #[serde(rename = "turn.failed")]
    TurnFailed(TurnFailedEvent),

    #[serde(rename = "item.started")]
    ItemStarted(ItemStartedEvent),

    #[serde(rename = "item.updated")]
    ItemUpdated(ItemUpdatedEvent),

    #[serde(rename = "item.completed")]
    ItemCompleted(ItemCompletedEvent),

    #[serde(rename = "error")]
    Error(ThreadErrorEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThreadStartedEvent {
    pub thread_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]

pub struct TurnStartedEvent {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TurnCompletedEvent {
    pub usage: Usage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TurnFailedEvent {
    pub error: ThreadErrorEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Usage {
    pub input_tokens: i64,

    pub cached_input_tokens: i64,

    pub output_tokens: i64,

    pub reasoning_output_tokens: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ItemStartedEvent {
    pub item: ThreadItem,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ItemCompletedEvent {
    pub item: ThreadItem,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ItemUpdatedEvent {
    pub item: ThreadItem,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThreadErrorEvent {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThreadItem {
    pub id: String,
    #[serde(flatten)]
    pub details: ThreadItemDetails,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ThreadItemDetails {
    AgentMessage(AgentMessageItem),

    Reasoning(ReasoningItem),

    CommandExecution(CommandExecutionItem),

    WebSearch(WebSearchItem),

    TodoList(TodoListItem),

    Error(ErrorItem),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentMessageItem {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReasoningItem {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CommandExecutionStatus {
    #[default]
    InProgress,
    Completed,
    Failed,
    Declined,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommandExecutionItem {
    pub command: String,
    pub aggregated_output: String,
    pub exit_code: Option<i32>,
    pub status: CommandExecutionStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileUpdateChange {
    pub path: String,
    pub kind: PatchChangeKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PatchChangeKind {
    Add,
    Delete,
    Update,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebSearchItem {
    pub id: String,
    pub query: String,
    pub action: WebSearchAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorItem {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TodoItem {
    pub text: String,
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TodoListItem {
    pub items: Vec<TodoItem>,
}
