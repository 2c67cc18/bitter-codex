use std::path::PathBuf;

use chrono::DateTime;
use chrono::Utc;
use codex_protocol::ThreadId;
use codex_protocol::dynamic_tools::DynamicToolSpec;
use codex_protocol::models::BaseInstructions;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::GitInfo;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::TokenUsage;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;

mod optional_option {
    use super::*;

    pub fn serialize<T, S>(value: &Option<Option<T>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        T: Serialize,
        S: Serializer,
    {
        match value {
            Some(value) => value.serialize(serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
    where
        T: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        Option::<T>::deserialize(deserializer).map(Some)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThreadEventPersistenceMode {
    #[default]
    Limited,

    Extended,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadPersistenceMetadata {
    pub cwd: Option<PathBuf>,

    pub model_provider: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateThreadParams {
    pub thread_id: ThreadId,

    pub forked_from_id: Option<ThreadId>,

    pub source: SessionSource,
    pub base_instructions: BaseInstructions,

    pub dynamic_tools: Vec<DynamicToolSpec>,

    pub metadata: ThreadPersistenceMetadata,

    pub event_persistence_mode: ThreadEventPersistenceMode,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResumeThreadParams {
    pub thread_id: ThreadId,

    pub rollout_path: Option<PathBuf>,

    pub history: Option<Vec<RolloutItem>>,

    pub include_archived: bool,

    pub metadata: ThreadPersistenceMetadata,

    pub event_persistence_mode: ThreadEventPersistenceMode,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppendThreadItemsParams {
    pub thread_id: ThreadId,

    pub items: Vec<RolloutItem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadThreadHistoryParams {
    pub thread_id: ThreadId,

    pub include_archived: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredThreadHistory {
    pub thread_id: ThreadId,

    pub items: Vec<RolloutItem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadThreadParams {
    pub thread_id: ThreadId,

    pub include_archived: bool,

    pub include_history: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadThreadByRolloutPathParams {
    pub rollout_path: PathBuf,

    pub include_archived: bool,

    pub include_history: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThreadSortKey {
    #[default]
    CreatedAt,

    UpdatedAt,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortDirection {
    Asc,

    #[default]
    Desc,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListThreadsParams {
    pub page_size: usize,

    pub cursor: Option<String>,

    pub sort_key: ThreadSortKey,

    pub sort_direction: SortDirection,

    pub allowed_sources: Vec<SessionSource>,

    pub model_providers: Option<Vec<String>>,

    pub cwd_filters: Option<Vec<PathBuf>>,

    pub archived: bool,

    pub search_term: Option<String>,

    pub use_state_db_only: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchThreadsParams {
    pub page_size: usize,

    pub cursor: Option<String>,

    pub sort_key: ThreadSortKey,

    pub sort_direction: SortDirection,

    pub allowed_sources: Vec<SessionSource>,

    pub archived: bool,

    pub search_term: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ThreadPage {
    pub items: Vec<StoredThread>,

    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredThreadSearchResult {
    pub thread: StoredThread,
    pub snippet: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ThreadSearchPage {
    pub items: Vec<StoredThreadSearchResult>,

    pub next_cursor: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StoredTurnItemsView {
    NotLoaded,

    #[default]
    Summary,

    Full,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StoredTurnStatus {
    Completed,

    Interrupted,

    Failed,

    InProgress,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredTurnError {
    pub message: String,

    pub additional_details: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListTurnsParams {
    pub thread_id: ThreadId,

    pub include_archived: bool,

    pub cursor: Option<String>,

    pub page_size: usize,

    pub sort_direction: SortDirection,

    pub items_view: StoredTurnItemsView,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredTurn {
    pub turn_id: String,

    pub items: Vec<RolloutItem>,

    pub items_view: StoredTurnItemsView,

    pub status: StoredTurnStatus,

    pub error: Option<StoredTurnError>,

    pub started_at: Option<i64>,

    pub completed_at: Option<i64>,

    pub duration_ms: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnPage {
    pub turns: Vec<StoredTurn>,

    pub next_cursor: Option<String>,

    pub backwards_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListItemsParams {
    pub thread_id: ThreadId,

    pub turn_id: String,

    pub include_archived: bool,

    pub cursor: Option<String>,

    pub page_size: usize,

    pub sort_direction: SortDirection,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ItemPage {
    pub items: Vec<RolloutItem>,

    pub next_cursor: Option<String>,

    pub backwards_cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredThread {
    pub thread_id: ThreadId,

    pub rollout_path: Option<PathBuf>,

    pub forked_from_id: Option<ThreadId>,

    pub preview: String,

    pub name: Option<String>,

    pub model_provider: String,

    pub model: Option<String>,

    pub reasoning_effort: Option<ReasoningEffort>,

    pub created_at: DateTime<Utc>,

    pub updated_at: DateTime<Utc>,

    pub archived_at: Option<DateTime<Utc>>,

    pub cwd: PathBuf,

    pub cli_version: String,

    pub source: SessionSource,

    pub git_info: Option<GitInfo>,

    pub token_usage: Option<TokenUsage>,

    pub first_user_message: Option<String>,

    pub history: Option<StoredThreadHistory>,
}

pub type ClearableField<T> = Option<Option<T>>;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitInfoPatch {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "optional_option"
    )]
    pub sha: ClearableField<String>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "optional_option"
    )]
    pub branch: ClearableField<String>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "optional_option"
    )]
    pub origin_url: ClearableField<String>,
}

impl GitInfoPatch {
    pub fn merge(&mut self, next: Self) {
        if next.sha.is_some() {
            self.sha = next.sha;
        }
        if next.branch.is_some() {
            self.branch = next.branch;
        }
        if next.origin_url.is_some() {
            self.origin_url = next.origin_url;
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ThreadMetadataPatch {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "optional_option"
    )]
    pub name: ClearableField<String>,

    pub rollout_path: Option<PathBuf>,

    pub preview: Option<String>,

    pub title: Option<String>,

    pub model_provider: Option<String>,

    pub model: Option<String>,

    pub reasoning_effort: Option<ReasoningEffort>,

    pub created_at: Option<DateTime<Utc>>,

    pub updated_at: Option<DateTime<Utc>>,

    pub source: Option<SessionSource>,
    pub cwd: Option<PathBuf>,

    pub cli_version: Option<String>,

    pub token_usage: Option<TokenUsage>,

    pub first_user_message: Option<String>,

    pub git_info: Option<GitInfoPatch>,
}

impl ThreadMetadataPatch {
    pub fn merge(&mut self, next: Self) {
        if next.name.is_some() {
            self.name = next.name;
        }
        if next.rollout_path.is_some() {
            self.rollout_path = next.rollout_path;
        }
        if next.preview.is_some() {
            self.preview = next.preview;
        }
        if next.title.is_some() {
            self.title = next.title;
        }
        if next.model_provider.is_some() {
            self.model_provider = next.model_provider;
        }
        if next.model.is_some() {
            self.model = next.model;
        }
        if next.reasoning_effort.is_some() {
            self.reasoning_effort = next.reasoning_effort;
        }
        if next.created_at.is_some() {
            self.created_at = next.created_at;
        }
        if next.updated_at.is_some() {
            self.updated_at = next.updated_at;
        }
        if next.source.is_some() {
            self.source = next.source;
        }
        if next.cwd.is_some() {
            self.cwd = next.cwd;
        }
        if next.cli_version.is_some() {
            self.cli_version = next.cli_version;
        }
        if next.token_usage.is_some() {
            self.token_usage = next.token_usage;
        }
        if next.first_user_message.is_some() {
            self.first_user_message = next.first_user_message;
        }
        if let Some(git_info) = next.git_info {
            self.git_info
                .get_or_insert_with(GitInfoPatch::default)
                .merge(git_info);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.rollout_path.is_none()
            && self.preview.is_none()
            && self.title.is_none()
            && self.model_provider.is_none()
            && self.model.is_none()
            && self.reasoning_effort.is_none()
            && self.created_at.is_none()
            && self.updated_at.is_none()
            && self.source.is_none()
            && self.cwd.is_none()
            && self.cli_version.is_none()
            && self.token_usage.is_none()
            && self.first_user_message.is_none()
            && self.git_info.is_none()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateThreadMetadataParams {
    pub thread_id: ThreadId,

    pub patch: ThreadMetadataPatch,

    pub include_archived: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveThreadParams {
    pub thread_id: ThreadId,
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::*;

    #[test]
    fn thread_metadata_patch_round_trips_optional_clears() {
        let patch = ThreadMetadataPatch {
            name: Some(None),
            ..Default::default()
        };

        let value = serde_json::to_value(&patch).expect("serialize patch");
        assert_eq!(value["name"], json!(null));

        let decoded: ThreadMetadataPatch =
            serde_json::from_value(value).expect("deserialize patch");
        assert_eq!(decoded.name, Some(None));
    }

    #[test]
    fn git_info_patch_round_trips_optional_clears() {
        let patch = ThreadMetadataPatch {
            git_info: Some(GitInfoPatch {
                sha: None,
                branch: Some(Some("main".to_string())),
                origin_url: Some(None),
            }),
            ..Default::default()
        };

        let value = serde_json::to_value(&patch).expect("serialize patch");
        assert_eq!(
            value["git_info"],
            json!({
                "branch": "main",
                "origin_url": null,
            })
        );

        let decoded: ThreadMetadataPatch =
            serde_json::from_value(value).expect("deserialize patch");
        assert_eq!(
            decoded.git_info,
            Some(GitInfoPatch {
                sha: None,
                branch: Some(Some("main".to_string())),
                origin_url: Some(None),
            })
        );
    }

    #[test]
    fn thread_metadata_patch_accepts_missing_fields() {
        let decoded: ThreadMetadataPatch =
            serde_json::from_value(json!({})).expect("deserialize empty patch");

        assert!(decoded.is_empty());
    }

    #[test]
    fn thread_metadata_patch_merge_uses_presence_semantics() {
        let mut current = ThreadMetadataPatch {
            name: Some(Some("old name".to_string())),
            preview: Some("old preview".to_string()),
            git_info: Some(GitInfoPatch {
                sha: Some(Some("abc123".to_string())),
                branch: Some(Some("main".to_string())),
                origin_url: None,
            }),
            ..Default::default()
        };

        current.merge(ThreadMetadataPatch {
            name: Some(None),
            preview: None,
            title: Some("new title".to_string()),
            git_info: Some(GitInfoPatch {
                sha: None,
                branch: Some(Some("feature".to_string())),
                origin_url: Some(None),
            }),
            ..Default::default()
        });

        assert_eq!(current.name, Some(None));
        assert_eq!(current.preview.as_deref(), Some("old preview"));
        assert_eq!(current.title.as_deref(), Some("new title"));
        assert_eq!(
            current.git_info,
            Some(GitInfoPatch {
                sha: Some(Some("abc123".to_string())),
                branch: Some(Some("feature".to_string())),
                origin_url: Some(None),
            })
        );
    }
}
