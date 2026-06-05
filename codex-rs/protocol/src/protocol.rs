use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fmt;
use std::ops::Mul;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use crate::SessionId;
use crate::ThreadId;
use crate::config_types::ReasoningSummary as ReasoningSummaryConfig;
use crate::dynamic_tools::DynamicToolCallOutputContentItem;
use crate::dynamic_tools::DynamicToolCallRequest;
use crate::dynamic_tools::DynamicToolResponse;
use crate::dynamic_tools::DynamicToolSpec;
use crate::items::TurnItem;
use crate::models::BaseInstructions;
use crate::models::ContentItem;
use crate::models::ResponseItem;
use crate::num_format::format_with_separators;
use crate::openai_models::ReasoningEffort as ReasoningEffortConfig;
use crate::parse_command::ParsedCommand;
use crate::user_input::UserInput;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_with::serde_as;
use strum_macros::Display;
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct W3cTraceContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traceparent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracestate: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct ThreadSettingsOverrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_roots: Option<Vec<AbsolutePathBuf>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<Option<ReasoningEffortConfig>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<ReasoningSummaryConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<Option<String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct TurnEnvironmentSelection {
    pub cwd: AbsolutePathBuf,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct GitSha(pub String);

impl GitSha {
    pub fn new(sha: &str) -> Self {
        Self(sha.to_string())
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdditionalContextKind {
    Untrusted,
    Application,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct AdditionalContextEntry {
    pub value: String,
    pub kind: AdditionalContextKind,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
#[non_exhaustive]
pub enum Op {
    Interrupt,

    CleanBackgroundTerminals,

    UserInput {
        items: Vec<UserInput>,

        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_id: Option<String>,

        #[serde(default, skip_serializing_if = "Option::is_none")]
        environments: Option<Vec<TurnEnvironmentSelection>>,

        #[serde(skip_serializing_if = "Option::is_none")]
        final_output_json_schema: Option<Value>,

        #[serde(default, skip_serializing_if = "Option::is_none")]
        responsesapi_client_metadata: Option<HashMap<String, String>>,

        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        additional_context: BTreeMap<String, AdditionalContextEntry>,

        #[serde(default, flatten)]
        thread_settings: ThreadSettingsOverrides,

        #[serde(default, skip_serializing_if = "Option::is_none")]
        web_tool_runtime: Option<WebToolRuntime>,
    },

    ThreadSettings {
        #[serde(flatten)]
        thread_settings: ThreadSettingsOverrides,
    },

    DynamicToolResponse {
        id: String,
        response: DynamicToolResponse,
    },

    ReloadUserConfig,

    Compact,

    Shutdown,
}

impl From<Vec<UserInput>> for Op {
    fn from(value: Vec<UserInput>) -> Self {
        Op::UserInput {
            environments: None,
            items: value,
            client_id: None,
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
            additional_context: BTreeMap::new(),
            thread_settings: ThreadSettingsOverrides::default(),
            web_tool_runtime: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WebToolRuntime {
    Hosted,
    Local,
    None,
}

impl Op {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Interrupt => "interrupt",
            Self::CleanBackgroundTerminals => "clean_background_terminals",
            Self::UserInput { .. } => "user_input",
            Self::ThreadSettings { .. } => "thread_settings",
            Self::DynamicToolResponse { .. } => "dynamic_tool_response",
            Self::ReloadUserConfig => "reload_user_config",
            Self::Compact => "compact",
            Self::Shutdown => "shutdown",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Event {
    pub id: String,

    pub msg: EventMsg,
}

#[derive(Debug, Clone, Deserialize, Serialize, Display)]
#[serde(tag = "type", rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum EventMsg {
    Error(ErrorEvent),

    Warning(WarningEvent),

    ModelReroute(ModelRerouteEvent),

    ModelVerification(ModelVerificationEvent),

    #[serde(rename = "task_started", alias = "turn_started")]
    TurnStarted(TurnStartedEvent),

    ThreadSettingsApplied(ThreadSettingsAppliedEvent),

    #[serde(rename = "task_complete", alias = "turn_complete")]
    TurnComplete(TurnCompleteEvent),

    TokenCount(TokenCountEvent),

    AgentReasoningSectionBreak(AgentReasoningSectionBreakEvent),

    SessionConfigured(SessionConfiguredEvent),

    ExecCommandBegin(ExecCommandBeginEvent),

    ExecCommandOutputDelta(ExecCommandOutputDeltaEvent),

    TerminalInteraction(TerminalInteractionEvent),

    ExecCommandEnd(ExecCommandEndEvent),

    DynamicToolCallRequest(DynamicToolCallRequest),

    DynamicToolCallResponse(DynamicToolCallResponseEvent),

    DeprecationNotice(DeprecationNoticeEvent),

    StreamError(StreamErrorEvent),

    TurnDiff(TurnDiffEvent),

    TurnAborted(TurnAbortedEvent),

    ShutdownComplete,

    RawResponseItem(RawResponseItemEvent),

    ItemStarted(ItemStartedEvent),
    ItemCompleted(ItemCompletedEvent),

    AgentMessageContentDelta(AgentMessageContentDeltaEvent),
    ReasoningContentDelta(ReasoningContentDeltaEvent),
    ReasoningRawContentDelta(ReasoningRawContentDeltaEvent),
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NonSteerableTurnKind {
    Compact,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexErrorInfo {
    ContextWindowExceeded,
    UsageLimitExceeded,
    ServerOverloaded,
    CyberPolicy,
    HttpConnectionFailed { http_status_code: Option<u16> },

    ResponseStreamConnectionFailed { http_status_code: Option<u16> },
    InternalServerError,
    Unauthorized,
    BadRequest,

    ResponseStreamDisconnected { http_status_code: Option<u16> },

    ResponseTooManyFailedAttempts { http_status_code: Option<u16> },

    ActiveTurnNotSteerable { turn_kind: NonSteerableTurnKind },
    Other,
}

impl CodexErrorInfo {
    pub fn affects_turn_status(&self) -> bool {
        match self {
            Self::ActiveTurnNotSteerable { .. } => false,
            Self::ContextWindowExceeded
            | Self::UsageLimitExceeded
            | Self::ServerOverloaded
            | Self::CyberPolicy
            | Self::HttpConnectionFailed { .. }
            | Self::ResponseStreamConnectionFailed { .. }
            | Self::InternalServerError
            | Self::Unauthorized
            | Self::BadRequest
            | Self::ResponseStreamDisconnected { .. }
            | Self::ResponseTooManyFailedAttempts { .. }
            | Self::Other => true,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RawResponseItemEvent {
    pub item: ResponseItem,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ItemStartedEvent {
    pub thread_id: ThreadId,
    pub turn_id: String,
    pub item: TurnItem,
    pub started_at_ms: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ItemCompletedEvent {
    pub thread_id: ThreadId,
    pub turn_id: String,
    pub item: TurnItem,

    #[serde(default = "default_item_completed_at_ms")]
    pub completed_at_ms: i64,
}

const fn default_item_completed_at_ms() -> i64 {
    0
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentMessageContentDeltaEvent {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub delta: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReasoningContentDeltaEvent {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub delta: String,

    #[serde(default)]
    pub summary_index: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReasoningRawContentDeltaEvent {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub delta: String,

    #[serde(default)]
    pub content_index: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ErrorEvent {
    pub message: String,
    #[serde(default)]
    pub codex_error_info: Option<CodexErrorInfo>,
}

impl ErrorEvent {
    pub fn affects_turn_status(&self) -> bool {
        self.codex_error_info
            .as_ref()
            .is_none_or(CodexErrorInfo::affects_turn_status)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WarningEvent {
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelRerouteReason {
    HighRiskCyberActivity,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ModelRerouteEvent {
    pub from_model: String,
    pub to_model: String,
    pub reason: ModelRerouteReason,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelVerification {
    TrustedAccessForCyber,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ModelVerificationEvent {
    pub verifications: Vec<ModelVerification>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TurnCompleteEvent {
    pub turn_id: String,
    pub last_agent_message: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_to_first_token_ms: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TurnStartedEvent {
    pub turn_id: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,

    pub model_context_window: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ThreadSettingsAppliedEvent {
    pub thread_settings: ThreadSettingsSnapshot,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ThreadSettingsSnapshot {
    pub model: String,
    pub model_provider_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    pub cwd: AbsolutePathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffortConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_summary: Option<ReasoningSummaryConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_output_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct TokenUsageInfo {
    pub total_token_usage: TokenUsage,
    pub last_token_usage: TokenUsage,

    pub model_context_window: Option<i64>,
}

impl TokenUsageInfo {
    pub fn new_or_append(
        info: &Option<TokenUsageInfo>,
        last: &Option<TokenUsage>,
        model_context_window: Option<i64>,
    ) -> Option<Self> {
        if info.is_none() && last.is_none() {
            return None;
        }

        let mut info = match info {
            Some(info) => info.clone(),
            None => Self {
                total_token_usage: TokenUsage::default(),
                last_token_usage: TokenUsage::default(),
                model_context_window,
            },
        };
        if let Some(last) = last {
            info.append_last_usage(last);
        }
        if let Some(model_context_window) = model_context_window {
            info.model_context_window = Some(model_context_window);
        }
        Some(info)
    }

    pub fn append_last_usage(&mut self, last: &TokenUsage) {
        self.total_token_usage.add_assign(last);
        self.last_token_usage = last.clone();
    }

    pub fn fill_to_context_window(&mut self, context_window: i64) {
        let previous_total = self.total_token_usage.total_tokens;
        let delta = (context_window - previous_total).max(0);

        self.model_context_window = Some(context_window);
        self.total_token_usage = TokenUsage {
            total_tokens: context_window,
            ..TokenUsage::default()
        };
        self.last_token_usage = TokenUsage {
            total_tokens: delta,
            ..TokenUsage::default()
        };
    }

    pub fn full_context_window(context_window: i64) -> Self {
        let mut info = Self {
            total_token_usage: TokenUsage::default(),
            last_token_usage: TokenUsage::default(),
            model_context_window: Some(context_window),
        };
        info.fill_to_context_window(context_window);
        info
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenCountEvent {
    pub info: Option<TokenUsageInfo>,
    pub rate_limits: Option<RateLimitSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RateLimitSnapshot {
    pub limit_id: Option<String>,
    pub limit_name: Option<String>,
    pub primary: Option<RateLimitWindow>,
    pub secondary: Option<RateLimitWindow>,
    pub credits: Option<CreditsSnapshot>,
    pub individual_limit: Option<SpendControlLimitSnapshot>,
    pub plan_type: Option<crate::account::PlanType>,
    pub rate_limit_reached_type: Option<RateLimitReachedType>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitReachedType {
    RateLimitReached,
    WorkspaceOwnerCreditsDepleted,
    WorkspaceMemberCreditsDepleted,
    WorkspaceOwnerUsageLimitReached,
    WorkspaceMemberUsageLimitReached,
}

impl FromStr for RateLimitReachedType {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "rate_limit_reached" => Ok(Self::RateLimitReached),
            "workspace_owner_credits_depleted" => Ok(Self::WorkspaceOwnerCreditsDepleted),
            "workspace_member_credits_depleted" => Ok(Self::WorkspaceMemberCreditsDepleted),
            "workspace_owner_usage_limit_reached" => Ok(Self::WorkspaceOwnerUsageLimitReached),
            "workspace_member_usage_limit_reached" => Ok(Self::WorkspaceMemberUsageLimitReached),
            other => Err(format!("unknown rate limit reached type: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RateLimitWindow {
    pub used_percent: f64,

    pub window_minutes: Option<i64>,

    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CreditsSnapshot {
    pub has_credits: bool,
    pub unlimited: bool,
    pub balance: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SpendControlLimitSnapshot {
    pub limit: String,
    pub used: String,
    pub remaining_percent: i32,
    pub resets_at: i64,
}

const BASELINE_TOKENS: i64 = 12000;

impl TokenUsage {
    pub fn is_zero(&self) -> bool {
        self.total_tokens == 0
    }

    pub fn cached_input(&self) -> i64 {
        self.cached_input_tokens.max(0)
    }

    pub fn non_cached_input(&self) -> i64 {
        (self.input_tokens - self.cached_input()).max(0)
    }

    pub fn blended_total(&self) -> i64 {
        (self.non_cached_input() + self.output_tokens.max(0)).max(0)
    }

    pub fn tokens_in_context_window(&self) -> i64 {
        self.total_tokens
    }

    pub fn percent_of_context_window_remaining(&self, context_window: i64) -> i64 {
        if context_window <= BASELINE_TOKENS {
            return 0;
        }

        let effective_window = context_window - BASELINE_TOKENS;
        let used = (self.tokens_in_context_window() - BASELINE_TOKENS).max(0);
        let remaining = (effective_window - used).max(0);
        ((remaining as f64 / effective_window as f64) * 100.0)
            .clamp(0.0, 100.0)
            .round() as i64
    }

    pub fn add_assign(&mut self, other: &TokenUsage) {
        self.input_tokens += other.input_tokens;
        self.cached_input_tokens += other.cached_input_tokens;
        self.output_tokens += other.output_tokens;
        self.reasoning_output_tokens += other.reasoning_output_tokens;
        self.total_tokens += other.total_tokens;
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FinalOutput {
    pub token_usage: TokenUsage,
}

impl From<TokenUsage> for FinalOutput {
    fn from(token_usage: TokenUsage) -> Self {
        Self { token_usage }
    }
}

impl fmt::Display for FinalOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let token_usage = &self.token_usage;

        write!(
            f,
            "Token usage: total={} input={}{} output={}{}",
            format_with_separators(token_usage.blended_total()),
            format_with_separators(token_usage.non_cached_input()),
            if token_usage.cached_input() > 0 {
                format!(
                    " (+ {} cached)",
                    format_with_separators(token_usage.cached_input())
                )
            } else {
                String::new()
            },
            format_with_separators(token_usage.output_tokens),
            if token_usage.reasoning_output_tokens > 0 {
                format!(
                    " (reasoning {})",
                    format_with_separators(token_usage.reasoning_output_tokens)
                )
            } else {
                String::new()
            }
        )
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentReasoningSectionBreakEvent {
    #[serde(default)]
    pub item_id: String,
    #[serde(default)]
    pub summary_index: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DynamicToolCallResponseEvent {
    pub call_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub completed_at_ms: i64,
    #[serde(default)]
    pub namespace: Option<String>,
    pub tool: String,
    pub arguments: serde_json::Value,
    pub content_items: Vec<DynamicToolCallOutputContentItem>,
    pub success: bool,
    pub error: Option<String>,
    pub duration: Duration,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConversationPathResponseEvent {
    pub conversation_id: ThreadId,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResumedHistory {
    pub conversation_id: ThreadId,
    pub history: Vec<RolloutItem>,
    pub rollout_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum InitialHistory {
    New,
    Cleared,
    Resumed(ResumedHistory),
    Forked(Vec<RolloutItem>),
}

impl InitialHistory {
    pub fn scan_rollout_items(&self, mut predicate: impl FnMut(&RolloutItem) -> bool) -> bool {
        match self {
            InitialHistory::New | InitialHistory::Cleared => false,
            InitialHistory::Resumed(resumed) => resumed.history.iter().any(&mut predicate),
            InitialHistory::Forked(items) => items.iter().any(predicate),
        }
    }

    pub fn forked_from_id(&self) -> Option<ThreadId> {
        match self {
            InitialHistory::New | InitialHistory::Cleared => None,
            InitialHistory::Resumed(resumed) => {
                resumed.history.iter().find_map(|item| match item {
                    RolloutItem::SessionMeta(meta_line) => meta_line.meta.forked_from_id,
                    _ => None,
                })
            }
            InitialHistory::Forked(items) => items.iter().find_map(|item| match item {
                RolloutItem::SessionMeta(meta_line) => Some(meta_line.meta.id),
                _ => None,
            }),
        }
    }

    pub fn session_cwd(&self) -> Option<PathBuf> {
        match self {
            InitialHistory::New | InitialHistory::Cleared => None,
            InitialHistory::Resumed(resumed) => session_cwd_from_items(&resumed.history),
            InitialHistory::Forked(items) => session_cwd_from_items(items),
        }
    }

    pub fn get_rollout_items(&self) -> Vec<RolloutItem> {
        match self {
            InitialHistory::New | InitialHistory::Cleared => Vec::new(),
            InitialHistory::Resumed(resumed) => resumed.history.clone(),
            InitialHistory::Forked(items) => items.clone(),
        }
    }

    pub fn get_event_msgs(&self) -> Option<Vec<EventMsg>> {
        match self {
            InitialHistory::New | InitialHistory::Cleared => None,
            InitialHistory::Resumed(resumed) => Some(
                resumed
                    .history
                    .iter()
                    .filter_map(|ri| match ri {
                        RolloutItem::EventMsg(ev) => Some(ev.clone()),
                        _ => None,
                    })
                    .collect(),
            ),
            InitialHistory::Forked(items) => Some(
                items
                    .iter()
                    .filter_map(|ri| match ri {
                        RolloutItem::EventMsg(ev) => Some(ev.clone()),
                        _ => None,
                    })
                    .collect(),
            ),
        }
    }

    pub fn get_base_instructions(&self) -> Option<BaseInstructions> {
        match self {
            InitialHistory::New | InitialHistory::Cleared => None,
            InitialHistory::Resumed(resumed) => {
                resumed.history.iter().find_map(|item| match item {
                    RolloutItem::SessionMeta(meta_line) => meta_line.meta.base_instructions.clone(),
                    _ => None,
                })
            }
            InitialHistory::Forked(items) => items.iter().find_map(|item| match item {
                RolloutItem::SessionMeta(meta_line) => meta_line.meta.base_instructions.clone(),
                _ => None,
            }),
        }
    }

    pub fn get_dynamic_tools(&self) -> Option<Vec<DynamicToolSpec>> {
        match self {
            InitialHistory::New | InitialHistory::Cleared => None,
            InitialHistory::Resumed(resumed) => {
                dynamic_tools_from_session_meta(resumed.history.as_slice())
            }
            InitialHistory::Forked(items) => dynamic_tools_from_session_meta(items.as_slice()),
        }
    }
}

fn session_cwd_from_items(items: &[RolloutItem]) -> Option<PathBuf> {
    items.iter().find_map(|item| match item {
        RolloutItem::SessionMeta(meta_line) => Some(meta_line.meta.cwd.clone()),
        _ => None,
    })
}

fn dynamic_tools_from_session_meta(items: &[RolloutItem]) -> Option<Vec<DynamicToolSpec>> {
    items.iter().find_map(|item| match item {
        RolloutItem::SessionMeta(meta_line) => meta_line.meta.dynamic_tools.clone(),
        _ => None,
    })
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SessionSource {
    Cli,
    #[default]
    VSCode,
    Exec,
    Custom(String),
    #[serde(other)]
    Unknown,
}

impl fmt::Display for SessionSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionSource::Cli => f.write_str("cli"),
            SessionSource::VSCode => f.write_str("vscode"),
            SessionSource::Exec => f.write_str("exec"),
            SessionSource::Custom(source) => f.write_str(source),
            SessionSource::Unknown => f.write_str("unknown"),
        }
    }
}

impl SessionSource {
    pub fn from_startup_arg(value: &str) -> Result<Self, &'static str> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err("session source must not be empty");
        }

        let normalized = trimmed.to_ascii_lowercase();
        Ok(match normalized.as_str() {
            "cli" => SessionSource::Cli,
            "vscode" => SessionSource::VSCode,
            "exec" => SessionSource::Exec,
            "unknown" => SessionSource::Unknown,
            _ => SessionSource::Custom(normalized),
        })
    }

    pub fn restriction_product(&self) -> Option<Product> {
        match self {
            SessionSource::Custom(source) => Product::from_session_source_name(source),
            SessionSource::Cli
            | SessionSource::VSCode
            | SessionSource::Exec
            | SessionSource::Unknown => Some(Product::Codex),
        }
    }

    pub fn matches_product_restriction(&self, products: &[Product]) -> bool {
        products.is_empty()
            || self
                .restriction_product()
                .is_some_and(|product| product.matches_product_restriction(products))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SessionMeta {
    pub id: ThreadId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forked_from_id: Option<ThreadId>,
    pub timestamp: String,
    pub cwd: PathBuf,
    pub originator: String,
    pub cli_version: String,
    #[serde(default)]
    pub source: SessionSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,

    pub base_instructions: Option<BaseInstructions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic_tools: Option<Vec<DynamicToolSpec>>,
}

impl Default for SessionMeta {
    fn default() -> Self {
        SessionMeta {
            id: ThreadId::default(),
            forked_from_id: None,
            timestamp: String::new(),
            cwd: PathBuf::new(),
            originator: String::new(),
            cli_version: String::new(),
            source: SessionSource::default(),
            model_provider: None,
            base_instructions: None,
            dynamic_tools: None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SessionMetaLine {
    #[serde(flatten)]
    pub meta: SessionMeta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git: Option<GitInfo>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum RolloutItem {
    SessionMeta(SessionMetaLine),
    ResponseItem(ResponseItem),
    Compacted(CompactedItem),
    TurnContext(TurnContextItem),
    EventMsg(EventMsg),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CompactedItem {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement_history: Option<Vec<ResponseItem>>,
}

impl From<CompactedItem> for ResponseItem {
    fn from(value: CompactedItem) -> Self {
        ResponseItem::Message {
            id: None,
            role: "assistant".to_string(),
            content: vec![ContentItem::OutputText {
                text: value.message,
            }],
            phase: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TurnContextItem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub cwd: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<ReasoningEffortConfig>,

    pub summary: ReasoningSummaryConfig,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "mode", content = "limit", rename_all = "snake_case")]
pub enum TruncationPolicy {
    Bytes(usize),
    Tokens(usize),
}

impl From<crate::openai_models::TruncationPolicyConfig> for TruncationPolicy {
    fn from(config: crate::openai_models::TruncationPolicyConfig) -> Self {
        match config.mode {
            crate::openai_models::TruncationMode::Bytes => Self::Bytes(config.limit as usize),
            crate::openai_models::TruncationMode::Tokens => Self::Tokens(config.limit as usize),
        }
    }
}

impl TruncationPolicy {
    pub fn token_budget(&self) -> usize {
        match self {
            TruncationPolicy::Bytes(bytes) => {
                usize::try_from(codex_utils_string::approx_tokens_from_byte_count(*bytes))
                    .unwrap_or(usize::MAX)
            }
            TruncationPolicy::Tokens(tokens) => *tokens,
        }
    }

    pub fn byte_budget(&self) -> usize {
        match self {
            TruncationPolicy::Bytes(bytes) => *bytes,
            TruncationPolicy::Tokens(tokens) => {
                codex_utils_string::approx_bytes_for_tokens(*tokens)
            }
        }
    }
}

impl Mul<f64> for TruncationPolicy {
    type Output = Self;

    fn mul(self, multiplier: f64) -> Self::Output {
        match self {
            TruncationPolicy::Bytes(bytes) => {
                TruncationPolicy::Bytes((bytes as f64 * multiplier).ceil() as usize)
            }
            TruncationPolicy::Tokens(tokens) => {
                TruncationPolicy::Tokens((tokens as f64 * multiplier).ceil() as usize)
            }
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct RolloutLine {
    pub timestamp: String,
    #[serde(flatten)]
    pub item: RolloutItem,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GitInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_hash: Option<GitSha>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_url: Option<String>,
}

#[derive(Debug, Clone, Copy, Display, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExecCommandSource {
    #[default]
    Agent,
    UnifiedExecStartup,
    UnifiedExecInteraction,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecCommandStatus {
    Completed,
    Failed,
    Declined,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExecCommandBeginEvent {
    pub call_id: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_id: Option<String>,

    pub turn_id: String,
    #[serde(default)]
    pub started_at_ms: i64,

    pub command: Vec<String>,

    pub cwd: AbsolutePathBuf,
    pub parsed_cmd: Vec<ParsedCommand>,

    #[serde(default)]
    pub source: ExecCommandSource,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interaction_input: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExecCommandEndEvent {
    pub call_id: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_id: Option<String>,

    pub turn_id: String,
    #[serde(default)]
    pub completed_at_ms: i64,

    pub command: Vec<String>,

    pub cwd: AbsolutePathBuf,
    pub parsed_cmd: Vec<ParsedCommand>,

    #[serde(default)]
    pub source: ExecCommandSource,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interaction_input: Option<String>,

    pub stdout: String,

    pub stderr: String,

    #[serde(default)]
    pub aggregated_output: String,

    pub exit_code: i32,

    pub duration: Duration,

    pub formatted_output: String,

    pub status: ExecCommandStatus,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExecOutputStream {
    Stdout,
    Stderr,
}

#[serde_as]
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ExecCommandOutputDeltaEvent {
    pub call_id: String,

    pub stream: ExecOutputStream,

    #[serde_as(as = "serde_with::base64::Base64")]
    pub chunk: Vec<u8>,
}

#[serde_as]
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct TerminalInteractionEvent {
    pub call_id: String,

    pub process_id: String,

    pub stdin: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeprecationNoticeEvent {
    pub summary: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StreamErrorEvent {
    pub message: String,
    #[serde(default)]
    pub codex_error_info: Option<CodexErrorInfo>,

    #[serde(default)]
    pub additional_details: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StreamInfoEvent {
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TurnDiffEvent {
    pub unified_diff: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Product {
    #[serde(alias = "CHATGPT")]
    Chatgpt,
    #[serde(alias = "CODEX")]
    Codex,
    #[serde(alias = "ATLAS")]
    Atlas,
}
impl Product {
    pub fn to_app_platform(self) -> &'static str {
        match self {
            Self::Chatgpt => "chat",
            Self::Codex => "codex",
            Self::Atlas => "atlas",
        }
    }

    pub fn from_session_source_name(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "chatgpt" => Some(Self::Chatgpt),
            "codex" => Some(Self::Codex),
            "atlas" => Some(Self::Atlas),
            _ => None,
        }
    }

    pub fn matches_product_restriction(&self, products: &[Product]) -> bool {
        products.is_empty() || products.contains(self)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SessionConfiguredEvent {
    pub session_id: SessionId,
    pub thread_id: ThreadId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forked_from_id: Option<ThreadId>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_name: Option<String>,

    pub model: String,

    pub model_provider_id: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,

    pub cwd: AbsolutePathBuf,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffortConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_messages: Option<Vec<EventMsg>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub rollout_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Chunk {
    pub orig_index: u32,
    pub deleted_lines: Vec<String>,
    pub inserted_lines: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TurnAbortedEvent {
    pub turn_id: Option<String>,
    pub reason: TurnAbortReason,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TurnAbortReason {
    Interrupted,
    Replaced,
    BudgetLimited,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::UserMessageItem;
    use anyhow::Result;
    use codex_utils_absolute_path::test_support::PathBufExt;
    use codex_utils_absolute_path::test_support::test_path_buf;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use tempfile::NamedTempFile;

    #[test]
    fn item_started_event_requires_started_at_ms() {
        let mut value = serde_json::to_value(ItemStartedEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-1".into(),
            item: TurnItem::UserMessage(UserMessageItem::new(&[])),
            started_at_ms: 123,
        })
        .unwrap();
        value.as_object_mut().unwrap().remove("started_at_ms");

        assert!(serde_json::from_value::<ItemStartedEvent>(value).is_err());
    }

    #[test]
    fn item_completed_event_defaults_missing_completed_at_ms() {
        let mut value = serde_json::to_value(ItemCompletedEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-1".into(),
            item: TurnItem::UserMessage(UserMessageItem::new(&[])),
            completed_at_ms: 123,
        })
        .unwrap();
        value.as_object_mut().unwrap().remove("completed_at_ms");

        let event = serde_json::from_value::<ItemCompletedEvent>(value).unwrap();
        assert_eq!(event.completed_at_ms, 0);
    }
    #[test]
    fn generic_error_affects_turn_status() {
        let event = ErrorEvent {
            message: "generic".into(),
            codex_error_info: Some(CodexErrorInfo::Other),
        };
        assert!(event.affects_turn_status());
    }

    #[test]
    fn user_input_serialization_omits_final_output_json_schema_when_none() -> Result<()> {
        let op = Op::UserInput {
            environments: None,
            items: Vec::new(),
            client_id: None,
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
            additional_context: BTreeMap::new(),
            thread_settings: Default::default(),
            web_tool_runtime: None,
        };

        let json_op = serde_json::to_value(op)?;
        assert_eq!(json_op, json!({ "type": "user_input", "items": [] }));

        Ok(())
    }

    #[test]
    fn user_input_deserializes_without_final_output_json_schema_field() -> Result<()> {
        let op: Op = serde_json::from_value(json!({ "type": "user_input", "items": [] }))?;

        assert_eq!(
            op,
            Op::UserInput {
                environments: None,
                items: Vec::new(),
                client_id: None,
                final_output_json_schema: None,
                responsesapi_client_metadata: None,
                additional_context: BTreeMap::new(),
                thread_settings: Default::default(),
                web_tool_runtime: None,
            }
        );

        Ok(())
    }

    #[test]
    fn initial_history_restores_dynamic_tools_from_session_meta() {
        let dynamic_tools = vec![DynamicToolSpec {
            namespace: Some("codex_app".to_string()),
            name: "geo_lookup".to_string(),
            description: "lookup a city".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "city": { "type": "string" }
                },
                "required": ["city"],
                "additionalProperties": false
            }),
            defer_loading: true,
        }];
        let history = vec![RolloutItem::SessionMeta(SessionMetaLine {
            meta: SessionMeta {
                id: ThreadId::new(),
                dynamic_tools: Some(dynamic_tools.clone()),
                ..Default::default()
            },
            git: None,
        })];

        assert_eq!(
            InitialHistory::Resumed(ResumedHistory {
                conversation_id: ThreadId::new(),
                history: history.clone(),
                rollout_path: None,
            })
            .get_dynamic_tools(),
            Some(dynamic_tools.clone())
        );
        assert_eq!(
            InitialHistory::Forked(history).get_dynamic_tools(),
            Some(dynamic_tools)
        );
        assert_eq!(InitialHistory::New.get_dynamic_tools(), None);
    }

    #[test]
    fn user_input_serialization_includes_final_output_json_schema_when_some() -> Result<()> {
        let schema = json!({
            "type": "object",
            "properties": {
                "answer": { "type": "string" }
            },
            "required": ["answer"],
            "additionalProperties": false
        });
        let op = Op::UserInput {
            environments: None,
            items: Vec::new(),
            client_id: None,
            final_output_json_schema: Some(schema.clone()),
            responsesapi_client_metadata: None,
            additional_context: BTreeMap::new(),
            thread_settings: Default::default(),
            web_tool_runtime: None,
        };

        let json_op = serde_json::to_value(op)?;
        assert_eq!(
            json_op,
            json!({
                "type": "user_input",
                "items": [],
                "final_output_json_schema": schema,
            })
        );

        Ok(())
    }

    #[test]
    fn user_input_with_responsesapi_client_metadata_round_trips() -> Result<()> {
        let op = Op::UserInput {
            environments: None,
            items: Vec::new(),
            client_id: None,
            final_output_json_schema: None,
            responsesapi_client_metadata: Some(HashMap::from([(
                "fiber_run_id".to_string(),
                "fiber-123".to_string(),
            )])),
            additional_context: BTreeMap::new(),
            thread_settings: Default::default(),
            web_tool_runtime: None,
        };

        let json_op = serde_json::to_value(&op)?;
        assert_eq!(
            json_op,
            json!({
                "type": "user_input",
                "items": [],
                "responsesapi_client_metadata": {
                    "fiber_run_id": "fiber-123",
                }
            })
        );
        assert_eq!(serde_json::from_value::<Op>(json_op)?, op);

        Ok(())
    }

    #[test]
    fn user_input_text_serializes_empty_text_elements() -> Result<()> {
        let input = UserInput::Text {
            text: "hello".to_string(),
            text_elements: Vec::new(),
        };

        let json_input = serde_json::to_value(input)?;
        assert_eq!(
            json_input,
            json!({
                "type": "text",
                "text": "hello",
                "text_elements": [],
            })
        );

        Ok(())
    }

    #[test]
    fn turn_aborted_event_deserializes_without_turn_id() -> Result<()> {
        let event: EventMsg = serde_json::from_value(json!({
            "type": "turn_aborted",
            "reason": "interrupted",
        }))?;

        match event {
            EventMsg::TurnAborted(TurnAbortedEvent {
                turn_id, reason, ..
            }) => {
                assert_eq!(turn_id, None);
                assert_eq!(reason, TurnAbortReason::Interrupted);
            }
            _ => panic!("expected turn_aborted event"),
        }

        Ok(())
    }

    #[test]
    fn serialize_event() -> Result<()> {
        let session_id = SessionId::from_string("67e55044-10b1-426f-9247-bb680e5fe0c7")?;
        let thread_id = ThreadId::from_string("67e55044-10b1-426f-9247-bb680e5fe0c8")?;
        let rollout_file = NamedTempFile::new()?;
        let event = Event {
            id: "1234".to_string(),
            msg: EventMsg::SessionConfigured(SessionConfiguredEvent {
                session_id,
                thread_id,
                forked_from_id: None,
                thread_name: None,
                model: "codex-mini-latest".to_string(),
                model_provider_id: "openai".to_string(),
                service_tier: None,
                cwd: test_path_buf("/home/user/project").abs(),
                reasoning_effort: Some(ReasoningEffortConfig::default()),
                initial_messages: None,
                rollout_path: Some(rollout_file.path().to_path_buf()),
            }),
        };

        let expected = json!({
            "id": "1234",
            "msg": {
                "type": "session_configured",
                "session_id": "67e55044-10b1-426f-9247-bb680e5fe0c7",
                "thread_id": "67e55044-10b1-426f-9247-bb680e5fe0c8",
                "model": "codex-mini-latest",
                "model_provider_id": "openai",
                "cwd": test_path_buf("/home/user/project"),
                "reasoning_effort": "medium",
                "rollout_path": format!("{}", rollout_file.path().display()),
            }
        });
        assert_eq!(expected, serde_json::to_value(&event)?);
        Ok(())
    }

    #[test]
    fn vec_u8_as_base64_serialization_and_deserialization() -> Result<()> {
        let event = ExecCommandOutputDeltaEvent {
            call_id: "call21".to_string(),
            stream: ExecOutputStream::Stdout,
            chunk: vec![1, 2, 3, 4, 5],
        };
        let serialized = serde_json::to_string(&event)?;
        assert_eq!(
            r#"{"call_id":"call21","stream":"stdout","chunk":"AQIDBAU="}"#,
            serialized,
        );

        let deserialized: ExecCommandOutputDeltaEvent = serde_json::from_str(&serialized)?;
        assert_eq!(deserialized, event);
        Ok(())
    }

    #[test]
    fn token_usage_info_new_or_append_updates_context_window_when_provided() {
        let initial = Some(TokenUsageInfo {
            total_token_usage: TokenUsage::default(),
            last_token_usage: TokenUsage::default(),
            model_context_window: Some(258_400),
        });
        let last = Some(TokenUsage {
            input_tokens: 10,
            cached_input_tokens: 0,
            output_tokens: 0,
            reasoning_output_tokens: 0,
            total_tokens: 10,
        });

        let info = TokenUsageInfo::new_or_append(&initial, &last, Some(128_000))
            .expect("new_or_append should return info");

        assert_eq!(info.model_context_window, Some(128_000));
    }

    #[test]
    fn token_usage_info_new_or_append_preserves_context_window_when_not_provided() {
        let initial = Some(TokenUsageInfo {
            total_token_usage: TokenUsage::default(),
            last_token_usage: TokenUsage::default(),
            model_context_window: Some(258_400),
        });
        let last = Some(TokenUsage {
            input_tokens: 10,
            cached_input_tokens: 0,
            output_tokens: 0,
            reasoning_output_tokens: 0,
            total_tokens: 10,
        });

        let info = TokenUsageInfo::new_or_append(&initial, &last, None)
            .expect("new_or_append should return info");

        assert_eq!(info.model_context_window, Some(258_400));
    }
}
