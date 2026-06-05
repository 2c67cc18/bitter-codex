use anyhow::Result;
use chrono::DateTime;
use chrono::Utc;
use codex_protocol::ThreadId;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::SessionSource;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    CreatedAt,

    UpdatedAt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    pub ts: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadsPage {
    pub items: Vec<ThreadMetadata>,

    pub next_anchor: Option<Anchor>,

    pub num_scanned_rows: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionOutcome {
    pub metadata: ThreadMetadata,

    pub parse_errors: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadMetadata {
    pub id: ThreadId,

    pub rollout_path: PathBuf,

    pub created_at: DateTime<Utc>,

    pub updated_at: DateTime<Utc>,

    pub source: String,

    pub model_provider: String,

    pub model: Option<String>,

    pub reasoning_effort: Option<ReasoningEffort>,

    pub cwd: PathBuf,

    pub cli_version: String,

    pub title: String,

    pub preview: Option<String>,

    pub tokens_used: i64,

    pub first_user_message: Option<String>,

    pub archived_at: Option<DateTime<Utc>>,

    pub git_sha: Option<String>,

    pub git_branch: Option<String>,

    pub git_origin_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadMetadataBuilder {
    pub id: ThreadId,

    pub rollout_path: PathBuf,

    pub created_at: DateTime<Utc>,

    pub updated_at: Option<DateTime<Utc>>,

    pub source: SessionSource,

    pub model_provider: Option<String>,

    pub cwd: PathBuf,

    pub cli_version: Option<String>,

    pub archived_at: Option<DateTime<Utc>>,

    pub git_sha: Option<String>,

    pub git_branch: Option<String>,

    pub git_origin_url: Option<String>,
}

impl ThreadMetadataBuilder {
    pub fn new(
        id: ThreadId,
        rollout_path: PathBuf,
        created_at: DateTime<Utc>,
        source: SessionSource,
    ) -> Self {
        Self {
            id,
            rollout_path,
            created_at,
            updated_at: None,
            source,
            model_provider: None,
            cwd: PathBuf::new(),
            cli_version: None,
            archived_at: None,
            git_sha: None,
            git_branch: None,
            git_origin_url: None,
        }
    }

    pub fn build(&self, default_provider: &str) -> ThreadMetadata {
        let source = crate::extract::enum_to_string(&self.source);
        let created_at = canonicalize_datetime(self.created_at);
        let updated_at = self
            .updated_at
            .map(canonicalize_datetime)
            .unwrap_or(created_at);
        ThreadMetadata {
            id: self.id,
            rollout_path: self.rollout_path.clone(),
            created_at,
            updated_at,
            source,
            model_provider: self
                .model_provider
                .clone()
                .unwrap_or_else(|| default_provider.to_string()),
            model: None,
            reasoning_effort: None,
            cwd: self.cwd.clone(),
            cli_version: self.cli_version.clone().unwrap_or_default(),
            title: String::new(),
            preview: None,
            tokens_used: 0,
            first_user_message: None,
            archived_at: self.archived_at.map(canonicalize_datetime),
            git_sha: self.git_sha.clone(),
            git_branch: self.git_branch.clone(),
            git_origin_url: self.git_origin_url.clone(),
        }
    }
}

impl ThreadMetadata {
    pub fn prefer_existing_git_info(&mut self, existing: &Self) {
        if existing.git_sha.is_some() {
            self.git_sha = existing.git_sha.clone();
        }
        if existing.git_branch.is_some() {
            self.git_branch = existing.git_branch.clone();
        }
        if existing.git_origin_url.is_some() {
            self.git_origin_url = existing.git_origin_url.clone();
        }
    }

    /// Preserve an existing user-facing title when reconciling rollout-derived metadata.
    pub fn prefer_existing_explicit_title(&mut self, existing: &Self) {
        let existing_title = existing.title.trim();
        if existing_title.is_empty()
            || existing.first_user_message.as_deref().map(str::trim) == Some(existing_title)
        {
            return;
        }

        let title = self.title.trim();
        if title.is_empty() || self.first_user_message.as_deref().map(str::trim) == Some(title) {
            self.title = existing.title.clone();
        }
    }

    pub fn diff_fields(&self, other: &Self) -> Vec<&'static str> {
        let mut diffs = Vec::new();
        if self.id != other.id {
            diffs.push("id");
        }
        if self.rollout_path != other.rollout_path {
            diffs.push("rollout_path");
        }
        if self.created_at != other.created_at {
            diffs.push("created_at");
        }
        if self.updated_at != other.updated_at {
            diffs.push("updated_at");
        }
        if self.source != other.source {
            diffs.push("source");
        }
        if self.model_provider != other.model_provider {
            diffs.push("model_provider");
        }
        if self.model != other.model {
            diffs.push("model");
        }
        if self.reasoning_effort != other.reasoning_effort {
            diffs.push("reasoning_effort");
        }
        if self.cwd != other.cwd {
            diffs.push("cwd");
        }
        if self.cli_version != other.cli_version {
            diffs.push("cli_version");
        }
        if self.title != other.title {
            diffs.push("title");
        }
        if self.preview != other.preview {
            diffs.push("preview");
        }
        if self.tokens_used != other.tokens_used {
            diffs.push("tokens_used");
        }
        if self.first_user_message != other.first_user_message {
            diffs.push("first_user_message");
        }
        if self.archived_at != other.archived_at {
            diffs.push("archived_at");
        }
        if self.git_sha != other.git_sha {
            diffs.push("git_sha");
        }
        if self.git_branch != other.git_branch {
            diffs.push("git_branch");
        }
        if self.git_origin_url != other.git_origin_url {
            diffs.push("git_origin_url");
        }
        diffs
    }
}

fn canonicalize_datetime(dt: DateTime<Utc>) -> DateTime<Utc> {
    epoch_millis_to_datetime(datetime_to_epoch_millis(dt)).unwrap_or(dt)
}

#[derive(Debug)]
pub(crate) struct ThreadRow {
    id: String,
    rollout_path: String,
    created_at: i64,
    updated_at: i64,
    source: String,
    model_provider: String,
    model: Option<String>,
    reasoning_effort: Option<String>,
    cwd: String,
    cli_version: String,
    title: String,
    preview: String,
    tokens_used: i64,
    first_user_message: String,
    archived_at: Option<i64>,
    git_sha: Option<String>,
    git_branch: Option<String>,
    git_origin_url: Option<String>,
}

impl ThreadRow {
    pub(crate) fn try_from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            rollout_path: row.try_get("rollout_path")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            source: row.try_get("source")?,
            model_provider: row.try_get("model_provider")?,
            model: row.try_get("model")?,
            reasoning_effort: row.try_get("reasoning_effort")?,
            cwd: row.try_get("cwd")?,
            cli_version: row.try_get("cli_version")?,
            title: row.try_get("title")?,
            preview: row.try_get("preview")?,
            tokens_used: row.try_get("tokens_used")?,
            first_user_message: row.try_get("first_user_message")?,
            archived_at: row.try_get("archived_at")?,
            git_sha: row.try_get("git_sha")?,
            git_branch: row.try_get("git_branch")?,
            git_origin_url: row.try_get("git_origin_url")?,
        })
    }
}

impl TryFrom<ThreadRow> for ThreadMetadata {
    type Error = anyhow::Error;

    fn try_from(row: ThreadRow) -> std::result::Result<Self, Self::Error> {
        let ThreadRow {
            id,
            rollout_path,
            created_at,
            updated_at,
            source,
            model_provider,
            model,
            reasoning_effort,
            cwd,
            cli_version,
            title,
            preview,
            tokens_used,
            first_user_message,
            archived_at,
            git_sha,
            git_branch,
            git_origin_url,
        } = row;
        Ok(Self {
            id: ThreadId::try_from(id)?,
            rollout_path: PathBuf::from(rollout_path),
            created_at: epoch_millis_to_datetime(created_at)?,
            updated_at: epoch_millis_to_datetime(updated_at)?,
            source,
            model_provider,
            model,
            reasoning_effort: reasoning_effort
                .and_then(|value| value.parse::<ReasoningEffort>().ok()),
            cwd: PathBuf::from(cwd),
            cli_version,
            title,
            preview: (!preview.is_empty()).then_some(preview),
            tokens_used,
            first_user_message: (!first_user_message.is_empty()).then_some(first_user_message),
            archived_at: archived_at.map(epoch_seconds_to_datetime).transpose()?,
            git_sha,
            git_branch,
            git_origin_url,
        })
    }
}

pub(crate) fn anchor_from_item(item: &ThreadMetadata, sort_key: SortKey) -> Option<Anchor> {
    let ts = match sort_key {
        SortKey::CreatedAt => item.created_at,
        SortKey::UpdatedAt => item.updated_at,
    };
    Some(Anchor { ts })
}

pub(crate) fn datetime_to_epoch_millis(dt: DateTime<Utc>) -> i64 {
    dt.timestamp_millis()
}

pub(crate) fn datetime_to_epoch_seconds(dt: DateTime<Utc>) -> i64 {
    dt.timestamp()
}

pub(crate) fn epoch_millis_to_datetime(value: i64) -> Result<DateTime<Utc>> {
    const MIN_EPOCH_MILLIS: i64 = 1_577_836_800_000;
    let millis = if value < MIN_EPOCH_MILLIS {
        value.saturating_mul(1000)
    } else {
        value
    };
    DateTime::<Utc>::from_timestamp_millis(millis)
        .ok_or_else(|| anyhow::anyhow!("invalid unix timestamp millis: {value}"))
}

pub(crate) fn epoch_seconds_to_datetime(value: i64) -> Result<DateTime<Utc>> {
    DateTime::<Utc>::from_timestamp(value, 0)
        .ok_or_else(|| anyhow::anyhow!("invalid unix timestamp seconds: {value}"))
}

#[derive(Debug, Clone)]
pub struct BackfillStats {
    pub scanned: usize,

    pub upserted: usize,

    pub failed: usize,
}

#[cfg(test)]
mod tests {
    use super::ThreadMetadata;
    use super::ThreadRow;
    use chrono::DateTime;
    use chrono::Utc;
    use codex_protocol::ThreadId;
    use codex_protocol::openai_models::ReasoningEffort;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    fn thread_row(reasoning_effort: Option<&str>) -> ThreadRow {
        ThreadRow {
            id: "00000000-0000-0000-0000-000000000123".to_string(),
            rollout_path: "/tmp/rollout-123.jsonl".to_string(),
            created_at: 1_700_000_000,
            updated_at: 1_700_000_100,
            source: "cli".to_string(),
            model_provider: "openai".to_string(),
            model: Some("gpt-5".to_string()),
            reasoning_effort: reasoning_effort.map(str::to_string),
            cwd: "/tmp/workspace".to_string(),
            cli_version: "0.0.0".to_string(),
            title: String::new(),
            preview: String::new(),
            tokens_used: 1,
            first_user_message: String::new(),
            archived_at: None,
            git_sha: None,
            git_branch: None,
            git_origin_url: None,
        }
    }

    fn expected_thread_metadata(reasoning_effort: Option<ReasoningEffort>) -> ThreadMetadata {
        ThreadMetadata {
            id: ThreadId::from_string("00000000-0000-0000-0000-000000000123")
                .expect("valid thread id"),
            rollout_path: PathBuf::from("/tmp/rollout-123.jsonl"),
            created_at: DateTime::<Utc>::from_timestamp(1_700_000_000, 0).expect("timestamp"),
            updated_at: DateTime::<Utc>::from_timestamp(1_700_000_100, 0).expect("timestamp"),
            source: "cli".to_string(),
            model_provider: "openai".to_string(),
            model: Some("gpt-5".to_string()),
            reasoning_effort,
            cwd: PathBuf::from("/tmp/workspace"),
            cli_version: "0.0.0".to_string(),
            title: String::new(),
            preview: None,
            tokens_used: 1,
            first_user_message: None,
            archived_at: None,
            git_sha: None,
            git_branch: None,
            git_origin_url: None,
        }
    }

    #[test]
    fn thread_row_parses_reasoning_effort() {
        let metadata = ThreadMetadata::try_from(thread_row(Some("high")))
            .expect("thread metadata should parse");

        assert_eq!(
            metadata,
            expected_thread_metadata(Some(ReasoningEffort::High))
        );
    }

    #[test]
    fn thread_row_ignores_unknown_reasoning_effort_values() {
        let metadata = ThreadMetadata::try_from(thread_row(Some("future")))
            .expect("thread metadata should parse");

        assert_eq!(metadata, expected_thread_metadata(None));
    }
}
