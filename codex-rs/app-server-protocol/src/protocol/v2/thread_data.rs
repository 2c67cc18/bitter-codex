use super::CodexErrorInfo;
use super::ThreadItem;
use super::ThreadStatus;
use super::TurnStatus;
use codex_protocol::protocol::SessionSource as CoreSessionSource;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde::Deserialize;
use serde::Serialize;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[derive(Default)]
pub enum SessionSource {
    Cli,
    #[serde(rename = "vscode")]
    #[default]
    VsCode,
    Exec,
    Custom(String),
    #[serde(other)]
    Unknown,
}

impl From<SessionSource> for CoreSessionSource {
    fn from(value: SessionSource) -> Self {
        match value {
            SessionSource::Cli => CoreSessionSource::Cli,
            SessionSource::VsCode => CoreSessionSource::VSCode,
            SessionSource::Exec => CoreSessionSource::Exec,
            SessionSource::Custom(source) => CoreSessionSource::Custom(source),
            SessionSource::Unknown => CoreSessionSource::Unknown,
        }
    }
}

impl From<CoreSessionSource> for SessionSource {
    fn from(value: CoreSessionSource) -> Self {
        match value {
            CoreSessionSource::Cli => SessionSource::Cli,
            CoreSessionSource::VSCode => SessionSource::VsCode,
            CoreSessionSource::Exec => SessionSource::Exec,
            CoreSessionSource::Custom(source) => SessionSource::Custom(source),
            CoreSessionSource::Unknown => SessionSource::Unknown,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitInfo {
    pub sha: Option<String>,
    pub branch: Option<String>,
    pub origin_url: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: String,

    pub session_id: String,

    pub forked_from_id: Option<String>,

    pub preview: String,

    pub ephemeral: bool,

    pub model_provider: String,

    pub created_at: i64,

    pub updated_at: i64,

    pub status: ThreadStatus,

    pub path: Option<PathBuf>,

    pub cwd: AbsolutePathBuf,

    pub cli_version: String,

    pub source: SessionSource,

    pub git_info: Option<GitInfo>,

    pub name: Option<String>,

    pub turns: Vec<Turn>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Turn {
    pub id: String,

    pub items: Vec<ThreadItem>,

    #[serde(default)]
    pub items_view: TurnItemsView,
    pub status: TurnStatus,

    pub error: Option<TurnError>,

    pub started_at: Option<i64>,

    pub completed_at: Option<i64>,

    pub duration_ms: Option<i64>,
}

#[derive(Default, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TurnItemsView {
    NotLoaded,

    Summary,

    #[default]
    Full,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Error)]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct TurnError {
    pub message: String,
    pub codex_error_info: Option<CodexErrorInfo>,
    #[serde(default)]
    pub additional_details: Option<String>,
}
