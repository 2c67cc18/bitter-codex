#![allow(warnings, clippy::all)]

use async_trait::async_trait;
use codex_utils_path as path_utils;
use std::cmp::Reverse;
use std::ffi::OsStr;
use std::io;
use std::ops::ControlFlow;
use std::path::Path;
use std::path::PathBuf;
use time::OffsetDateTime;
use time::PrimitiveDateTime;
use time::format_description::FormatItem;
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use uuid::Uuid;

use super::ARCHIVED_SESSIONS_SUBDIR;
use super::SESSIONS_SUBDIR;
use crate::protocol::EventMsg;
use crate::state_db;
use codex_protocol::ThreadId;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::protocol::RolloutLine;
use codex_protocol::protocol::SessionMetaLine;
use codex_protocol::protocol::SessionSource;

const USER_MESSAGE_BEGIN: &str = "## My request for Codex:";

#[derive(Debug, Default, PartialEq)]
pub struct ThreadsPage {
    pub items: Vec<ThreadItem>,

    pub next_cursor: Option<Cursor>,

    pub num_scanned_files: usize,

    pub reached_scan_cap: bool,
}

#[derive(Debug, PartialEq, Default)]
pub struct ThreadItem {
    pub path: PathBuf,

    pub thread_id: Option<ThreadId>,

    pub first_user_message: Option<String>,

    pub preview: Option<String>,

    pub cwd: Option<PathBuf>,

    pub git_branch: Option<String>,

    pub git_sha: Option<String>,

    pub git_origin_url: Option<String>,

    pub source: Option<SessionSource>,

    pub model_provider: Option<String>,

    pub cli_version: Option<String>,

    pub created_at: Option<String>,

    pub updated_at: Option<String>,
}

#[allow(dead_code)]
#[deprecated(note = "use ThreadItem")]
pub type ConversationItem = ThreadItem;
#[allow(dead_code)]
#[deprecated(note = "use ThreadsPage")]
pub type ConversationsPage = ThreadsPage;

#[derive(Default)]
struct HeadTailSummary {
    saw_session_meta: bool,
    thread_id: Option<ThreadId>,
    first_user_message: Option<String>,
    preview: Option<String>,
    cwd: Option<PathBuf>,
    git_branch: Option<String>,
    git_sha: Option<String>,
    git_origin_url: Option<String>,
    source: Option<SessionSource>,
    model_provider: Option<String>,
    cli_version: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

const MAX_SCAN_FILES: usize = 10000;
const HEAD_RECORD_LIMIT: usize = 10;
const USER_EVENT_SCAN_LIMIT: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadSortKey {
    CreatedAt,
    UpdatedAt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadListLayout {
    NestedByDate,
    Flat,
}

pub struct ThreadListConfig<'a> {
    pub allowed_sources: &'a [SessionSource],
    pub model_providers: Option<&'a [String]>,
    pub cwd_filters: Option<&'a [PathBuf]>,
    pub default_provider: &'a str,
    pub layout: ThreadListLayout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    ts: OffsetDateTime,
}

impl Cursor {
    fn new(ts: OffsetDateTime) -> Self {
        Self { ts }
    }

    pub(crate) fn timestamp(&self) -> OffsetDateTime {
        self.ts
    }
}

struct AnchorState {
    ts: OffsetDateTime,
    passed: bool,
}

impl AnchorState {
    fn new(anchor: Option<Cursor>) -> Self {
        match anchor {
            Some(cursor) => Self {
                ts: cursor.ts,
                passed: false,
            },
            None => Self {
                ts: OffsetDateTime::UNIX_EPOCH,
                passed: true,
            },
        }
    }

    fn should_skip(&mut self, ts: OffsetDateTime, _id: Uuid) -> bool {
        if self.passed {
            return false;
        }
        if ts < self.ts {
            self.passed = true;
            false
        } else {
            true
        }
    }
}

#[async_trait]
trait RolloutFileVisitor {
    async fn visit(
        &mut self,
        ts: OffsetDateTime,
        id: Uuid,
        path: PathBuf,
        scanned: usize,
    ) -> ControlFlow<()>;
}

struct FilesByCreatedAtVisitor<'a> {
    items: &'a mut Vec<ThreadItem>,
    page_size: usize,
    anchor_state: AnchorState,
    more_matches_available: bool,
    allowed_sources: &'a [SessionSource],
    provider_matcher: Option<&'a ProviderMatcher<'a>>,
    cwd_filters: Option<&'a [PathBuf]>,
}

#[async_trait]
impl<'a> RolloutFileVisitor for FilesByCreatedAtVisitor<'a> {
    async fn visit(
        &mut self,
        ts: OffsetDateTime,
        id: Uuid,
        path: PathBuf,
        scanned: usize,
    ) -> ControlFlow<()> {
        if scanned >= MAX_SCAN_FILES && self.items.len() >= self.page_size {
            self.more_matches_available = true;
            return ControlFlow::Break(());
        }
        if self.anchor_state.should_skip(ts, id) {
            return ControlFlow::Continue(());
        }
        if self.items.len() == self.page_size {
            self.more_matches_available = true;
            return ControlFlow::Break(());
        }
        let updated_at = file_modified_time(&path)
            .await
            .unwrap_or(None)
            .and_then(format_rfc3339);
        if let Some(item) = build_thread_item(
            path,
            self.allowed_sources,
            self.provider_matcher,
            self.cwd_filters,
            updated_at,
        )
        .await
        {
            self.items.push(item);
        }
        ControlFlow::Continue(())
    }
}

struct FilesByUpdatedAtVisitor<'a> {
    candidates: &'a mut Vec<ThreadCandidate>,
}

#[async_trait]
impl<'a> RolloutFileVisitor for FilesByUpdatedAtVisitor<'a> {
    async fn visit(
        &mut self,
        _ts: OffsetDateTime,
        id: Uuid,
        path: PathBuf,
        _scanned: usize,
    ) -> ControlFlow<()> {
        let updated_at = file_modified_time(&path).await.unwrap_or(None);
        self.candidates.push(ThreadCandidate {
            path,
            id,
            updated_at,
        });
        ControlFlow::Continue(())
    }
}

impl serde::Serialize for Cursor {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let ts_str = self
            .ts
            .format(&Rfc3339)
            .map_err(|e| serde::ser::Error::custom(format!("format error: {e}")))?;
        serializer.serialize_str(&ts_str)
    }
}

impl<'de> serde::Deserialize<'de> for Cursor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        parse_cursor(&s).ok_or_else(|| serde::de::Error::custom("invalid cursor"))
    }
}

impl From<codex_state::Anchor> for Cursor {
    fn from(anchor: codex_state::Anchor) -> Self {
        let ts = anchor
            .ts
            .timestamp_nanos_opt()
            .and_then(|nanos| OffsetDateTime::from_unix_timestamp_nanos(nanos as i128).ok())
            .unwrap_or(OffsetDateTime::UNIX_EPOCH);
        Self::new(ts)
    }
}

pub async fn get_threads(
    codex_home: &Path,
    page_size: usize,
    cursor: Option<&Cursor>,
    sort_key: ThreadSortKey,
    allowed_sources: &[SessionSource],
    model_providers: Option<&[String]>,
    cwd_filters: Option<&[PathBuf]>,
    default_provider: &str,
) -> io::Result<ThreadsPage> {
    let root = codex_home.join(SESSIONS_SUBDIR);
    get_threads_in_root(
        root,
        page_size,
        cursor,
        sort_key,
        ThreadListConfig {
            allowed_sources,
            model_providers,
            cwd_filters,
            default_provider,
            layout: ThreadListLayout::NestedByDate,
        },
    )
    .await
}

pub async fn get_threads_in_root(
    root: PathBuf,
    page_size: usize,
    cursor: Option<&Cursor>,
    sort_key: ThreadSortKey,
    config: ThreadListConfig<'_>,
) -> io::Result<ThreadsPage> {
    if !root.exists() {
        return Ok(ThreadsPage {
            items: Vec::new(),
            next_cursor: None,
            num_scanned_files: 0,
            reached_scan_cap: false,
        });
    }

    let anchor = cursor.cloned();

    let provider_matcher = config
        .model_providers
        .and_then(|filters| ProviderMatcher::new(filters, config.default_provider));

    let result = match config.layout {
        ThreadListLayout::NestedByDate => {
            traverse_directories_for_paths(
                root.clone(),
                page_size,
                anchor,
                sort_key,
                config.allowed_sources,
                provider_matcher.as_ref(),
                config.cwd_filters,
            )
            .await?
        }
        ThreadListLayout::Flat => {
            traverse_flat_paths(
                root.clone(),
                page_size,
                anchor,
                sort_key,
                config.allowed_sources,
                provider_matcher.as_ref(),
                config.cwd_filters,
            )
            .await?
        }
    };
    Ok(result)
}

async fn traverse_directories_for_paths(
    root: PathBuf,
    page_size: usize,
    anchor: Option<Cursor>,
    sort_key: ThreadSortKey,
    allowed_sources: &[SessionSource],
    provider_matcher: Option<&ProviderMatcher<'_>>,
    cwd_filters: Option<&[PathBuf]>,
) -> io::Result<ThreadsPage> {
    match sort_key {
        ThreadSortKey::CreatedAt => {
            traverse_directories_for_paths_created(
                root,
                page_size,
                anchor,
                allowed_sources,
                provider_matcher,
                cwd_filters,
            )
            .await
        }
        ThreadSortKey::UpdatedAt => {
            traverse_directories_for_paths_updated(
                root,
                page_size,
                anchor,
                allowed_sources,
                provider_matcher,
                cwd_filters,
            )
            .await
        }
    }
}

async fn traverse_flat_paths(
    root: PathBuf,
    page_size: usize,
    anchor: Option<Cursor>,
    sort_key: ThreadSortKey,
    allowed_sources: &[SessionSource],
    provider_matcher: Option<&ProviderMatcher<'_>>,
    cwd_filters: Option<&[PathBuf]>,
) -> io::Result<ThreadsPage> {
    match sort_key {
        ThreadSortKey::CreatedAt => {
            traverse_flat_paths_created(
                root,
                page_size,
                anchor,
                allowed_sources,
                provider_matcher,
                cwd_filters,
            )
            .await
        }
        ThreadSortKey::UpdatedAt => {
            traverse_flat_paths_updated(
                root,
                page_size,
                anchor,
                allowed_sources,
                provider_matcher,
                cwd_filters,
            )
            .await
        }
    }
}

async fn traverse_directories_for_paths_created(
    root: PathBuf,
    page_size: usize,
    anchor: Option<Cursor>,
    allowed_sources: &[SessionSource],
    provider_matcher: Option<&ProviderMatcher<'_>>,
    cwd_filters: Option<&[PathBuf]>,
) -> io::Result<ThreadsPage> {
    let mut items: Vec<ThreadItem> = Vec::with_capacity(page_size);
    let mut scanned_files = 0usize;
    let mut more_matches_available = false;
    let mut visitor = FilesByCreatedAtVisitor {
        items: &mut items,
        page_size,
        anchor_state: AnchorState::new(anchor),
        more_matches_available,
        allowed_sources,
        provider_matcher,
        cwd_filters,
    };
    walk_rollout_files(&root, &mut scanned_files, &mut visitor).await?;
    more_matches_available = visitor.more_matches_available;

    let reached_scan_cap = scanned_files >= MAX_SCAN_FILES;
    if reached_scan_cap && !items.is_empty() {
        more_matches_available = true;
    }

    let next = if more_matches_available {
        build_next_cursor(&items, ThreadSortKey::CreatedAt)
    } else {
        None
    };
    Ok(ThreadsPage {
        items,
        next_cursor: next,
        num_scanned_files: scanned_files,
        reached_scan_cap,
    })
}

async fn traverse_directories_for_paths_updated(
    root: PathBuf,
    page_size: usize,
    anchor: Option<Cursor>,
    allowed_sources: &[SessionSource],
    provider_matcher: Option<&ProviderMatcher<'_>>,
    cwd_filters: Option<&[PathBuf]>,
) -> io::Result<ThreadsPage> {
    let mut items: Vec<ThreadItem> = Vec::with_capacity(page_size);
    let mut scanned_files = 0usize;
    let mut anchor_state = AnchorState::new(anchor);
    let mut more_matches_available = false;

    let mut candidates = collect_files_by_updated_at(&root, &mut scanned_files).await?;
    candidates.sort_by_key(|candidate| {
        let ts = candidate.updated_at.unwrap_or(OffsetDateTime::UNIX_EPOCH);
        (Reverse(ts), Reverse(candidate.id))
    });

    for candidate in candidates.into_iter() {
        let ts = candidate.updated_at.unwrap_or(OffsetDateTime::UNIX_EPOCH);
        if anchor_state.should_skip(ts, candidate.id) {
            continue;
        }
        if items.len() == page_size {
            more_matches_available = true;
            break;
        }

        let updated_at_fallback = candidate.updated_at.and_then(format_rfc3339);
        if let Some(item) = build_thread_item(
            candidate.path,
            allowed_sources,
            provider_matcher,
            cwd_filters,
            updated_at_fallback,
        )
        .await
        {
            items.push(item);
        }
    }

    let reached_scan_cap = scanned_files >= MAX_SCAN_FILES;
    if reached_scan_cap && !items.is_empty() {
        more_matches_available = true;
    }

    let next = if more_matches_available {
        build_next_cursor(&items, ThreadSortKey::UpdatedAt)
    } else {
        None
    };
    Ok(ThreadsPage {
        items,
        next_cursor: next,
        num_scanned_files: scanned_files,
        reached_scan_cap,
    })
}

async fn traverse_flat_paths_created(
    root: PathBuf,
    page_size: usize,
    anchor: Option<Cursor>,
    allowed_sources: &[SessionSource],
    provider_matcher: Option<&ProviderMatcher<'_>>,
    cwd_filters: Option<&[PathBuf]>,
) -> io::Result<ThreadsPage> {
    let mut items: Vec<ThreadItem> = Vec::with_capacity(page_size);
    let mut scanned_files = 0usize;
    let mut anchor_state = AnchorState::new(anchor);
    let mut more_matches_available = false;

    let files = collect_flat_rollout_files(&root, &mut scanned_files).await?;
    for (ts, id, path) in files.into_iter() {
        if anchor_state.should_skip(ts, id) {
            continue;
        }
        if items.len() == page_size {
            more_matches_available = true;
            break;
        }
        let updated_at = file_modified_time(&path)
            .await
            .unwrap_or(None)
            .and_then(format_rfc3339);
        if let Some(item) = build_thread_item(
            path,
            allowed_sources,
            provider_matcher,
            cwd_filters,
            updated_at,
        )
        .await
        {
            items.push(item);
        }
    }

    let reached_scan_cap = scanned_files >= MAX_SCAN_FILES;
    if reached_scan_cap && !items.is_empty() {
        more_matches_available = true;
    }

    let next = if more_matches_available {
        build_next_cursor(&items, ThreadSortKey::CreatedAt)
    } else {
        None
    };
    Ok(ThreadsPage {
        items,
        next_cursor: next,
        num_scanned_files: scanned_files,
        reached_scan_cap,
    })
}

async fn traverse_flat_paths_updated(
    root: PathBuf,
    page_size: usize,
    anchor: Option<Cursor>,
    allowed_sources: &[SessionSource],
    provider_matcher: Option<&ProviderMatcher<'_>>,
    cwd_filters: Option<&[PathBuf]>,
) -> io::Result<ThreadsPage> {
    let mut items: Vec<ThreadItem> = Vec::with_capacity(page_size);
    let mut scanned_files = 0usize;
    let mut anchor_state = AnchorState::new(anchor);
    let mut more_matches_available = false;

    let mut candidates = collect_flat_files_by_updated_at(&root, &mut scanned_files).await?;
    candidates.sort_by_key(|candidate| {
        let ts = candidate.updated_at.unwrap_or(OffsetDateTime::UNIX_EPOCH);
        (Reverse(ts), Reverse(candidate.id))
    });

    for candidate in candidates.into_iter() {
        let ts = candidate.updated_at.unwrap_or(OffsetDateTime::UNIX_EPOCH);
        if anchor_state.should_skip(ts, candidate.id) {
            continue;
        }
        if items.len() == page_size {
            more_matches_available = true;
            break;
        }

        let updated_at_fallback = candidate.updated_at.and_then(format_rfc3339);
        if let Some(item) = build_thread_item(
            candidate.path,
            allowed_sources,
            provider_matcher,
            cwd_filters,
            updated_at_fallback,
        )
        .await
        {
            items.push(item);
        }
    }

    let reached_scan_cap = scanned_files >= MAX_SCAN_FILES;
    if reached_scan_cap && !items.is_empty() {
        more_matches_available = true;
    }

    let next = if more_matches_available {
        build_next_cursor(&items, ThreadSortKey::UpdatedAt)
    } else {
        None
    };
    Ok(ThreadsPage {
        items,
        next_cursor: next,
        num_scanned_files: scanned_files,
        reached_scan_cap,
    })
}

pub fn parse_cursor(token: &str) -> Option<Cursor> {
    if token.contains('|') {
        return None;
    }

    let ts = OffsetDateTime::parse(token, &Rfc3339).ok().or_else(|| {
        let format: &[FormatItem] =
            format_description!("[year]-[month]-[day]T[hour]-[minute]-[second]");
        PrimitiveDateTime::parse(token, format)
            .ok()
            .map(PrimitiveDateTime::assume_utc)
    })?;

    Some(Cursor::new(ts))
}

fn build_next_cursor(items: &[ThreadItem], sort_key: ThreadSortKey) -> Option<Cursor> {
    let last = items.last()?;
    let file_name = last.path.file_name()?.to_string_lossy();
    let (created_ts, _id) = parse_timestamp_uuid_from_filename(&file_name)?;
    let ts = match sort_key {
        ThreadSortKey::CreatedAt => created_ts,
        ThreadSortKey::UpdatedAt => {
            let updated_at = last.updated_at.as_deref()?;
            OffsetDateTime::parse(updated_at, &Rfc3339).ok()?
        }
    };
    Some(Cursor::new(ts))
}

async fn build_thread_item(
    path: PathBuf,
    allowed_sources: &[SessionSource],
    provider_matcher: Option<&ProviderMatcher<'_>>,
    cwd_filters: Option<&[PathBuf]>,
    updated_at: Option<String>,
) -> Option<ThreadItem> {
    let summary = read_head_summary(&path, HEAD_RECORD_LIMIT)
        .await
        .unwrap_or_default();
    if !allowed_sources.is_empty()
        && !summary
            .source
            .as_ref()
            .is_some_and(|source| allowed_sources.contains(source))
    {
        return None;
    }
    if let Some(matcher) = provider_matcher
        && !matcher.matches(summary.model_provider.as_deref())
    {
        return None;
    }
    if let Some(cwd_filters) = cwd_filters
        && !summary.cwd.as_ref().is_some_and(|cwd| {
            cwd_filters
                .iter()
                .any(|filter| path_utils::paths_match_after_normalization(cwd, filter))
        })
    {
        return None;
    }

    if summary.saw_session_meta && summary.preview.is_some() {
        let HeadTailSummary {
            thread_id,
            first_user_message,
            preview,
            cwd,
            git_branch,
            git_sha,
            git_origin_url,
            source,
            model_provider,
            cli_version,
            created_at,
            updated_at: mut summary_updated_at,
            ..
        } = summary;
        if summary_updated_at.is_none() {
            summary_updated_at = updated_at.or_else(|| created_at.clone());
        }
        return Some(ThreadItem {
            path,
            thread_id,
            first_user_message,
            preview,
            cwd,
            git_branch,
            git_sha,
            git_origin_url,
            source,
            model_provider,
            cli_version,
            created_at,
            updated_at: summary_updated_at,
        });
    }
    None
}

pub async fn read_thread_item_from_rollout(path: PathBuf) -> Option<ThreadItem> {
    build_thread_item(path, &[], None, None, None).await
}

async fn collect_dirs_desc<T, F>(parent: &Path, parse: F) -> io::Result<Vec<(T, PathBuf)>>
where
    T: Ord + Copy,
    F: Fn(&str) -> Option<T>,
{
    let mut dir = tokio::fs::read_dir(parent).await?;
    let mut vec: Vec<(T, PathBuf)> = Vec::new();
    while let Some(entry) = dir.next_entry().await? {
        if entry
            .file_type()
            .await
            .map(|ft| ft.is_dir())
            .unwrap_or(false)
            && let Some(s) = entry.file_name().to_str()
            && let Some(v) = parse(s)
        {
            vec.push((v, entry.path()));
        }
    }
    vec.sort_by_key(|(v, _)| Reverse(*v));
    Ok(vec)
}

async fn collect_files<T, F>(parent: &Path, parse: F) -> io::Result<Vec<T>>
where
    F: Fn(&str, &Path) -> Option<T>,
{
    let mut dir = tokio::fs::read_dir(parent).await?;
    let mut collected: Vec<T> = Vec::new();
    while let Some(entry) = dir.next_entry().await? {
        if entry
            .file_type()
            .await
            .map(|ft| ft.is_file())
            .unwrap_or(false)
            && let Some(s) = entry.file_name().to_str()
            && let Some(v) = parse(s, &entry.path())
        {
            collected.push(v);
        }
    }
    Ok(collected)
}

async fn collect_flat_rollout_files(
    root: &Path,
    scanned_files: &mut usize,
) -> io::Result<Vec<(OffsetDateTime, Uuid, PathBuf)>> {
    let mut dir = tokio::fs::read_dir(root).await?;
    let mut collected = Vec::new();
    while let Some(entry) = dir.next_entry().await? {
        if *scanned_files >= MAX_SCAN_FILES {
            break;
        }
        if !entry
            .file_type()
            .await
            .map(|ft| ft.is_file())
            .unwrap_or(false)
        {
            continue;
        }
        let file_name = entry.file_name();
        let Some(name_str) = file_name.to_str() else {
            continue;
        };
        if !name_str.starts_with("rollout-") || !name_str.ends_with(".jsonl") {
            continue;
        }
        let Some((ts, id)) = parse_timestamp_uuid_from_filename(name_str) else {
            continue;
        };
        *scanned_files += 1;
        if *scanned_files > MAX_SCAN_FILES {
            break;
        }
        collected.push((ts, id, entry.path()));
    }
    collected.sort_by_key(|(ts, sid, _path)| (Reverse(*ts), Reverse(*sid)));
    Ok(collected)
}

async fn collect_rollout_day_files(
    day_path: &Path,
) -> io::Result<Vec<(OffsetDateTime, Uuid, PathBuf)>> {
    let mut day_files = collect_files(day_path, |name_str, path| {
        if !name_str.starts_with("rollout-") || !name_str.ends_with(".jsonl") {
            return None;
        }

        parse_timestamp_uuid_from_filename(name_str).map(|(ts, id)| (ts, id, path.to_path_buf()))
    })
    .await?;

    day_files.sort_by_key(|(ts, sid, _path)| (Reverse(*ts), Reverse(*sid)));
    Ok(day_files)
}

pub(crate) fn parse_timestamp_uuid_from_filename(name: &str) -> Option<(OffsetDateTime, Uuid)> {
    let core = name.strip_prefix("rollout-")?.strip_suffix(".jsonl")?;

    let (sep_idx, uuid) = core
        .match_indices('-')
        .rev()
        .find_map(|(i, _)| Uuid::parse_str(&core[i + 1..]).ok().map(|u| (i, u)))?;

    let ts_str = &core[..sep_idx];
    let format: &[FormatItem] =
        format_description!("[year]-[month]-[day]T[hour]-[minute]-[second]");
    let ts = PrimitiveDateTime::parse(ts_str, format).ok()?.assume_utc();
    Some((ts, uuid))
}

struct ThreadCandidate {
    path: PathBuf,
    id: Uuid,
    updated_at: Option<OffsetDateTime>,
}

async fn collect_files_by_updated_at(
    root: &Path,
    scanned_files: &mut usize,
) -> io::Result<Vec<ThreadCandidate>> {
    let mut candidates = Vec::new();
    let mut visitor = FilesByUpdatedAtVisitor {
        candidates: &mut candidates,
    };
    walk_rollout_files(root, scanned_files, &mut visitor).await?;

    Ok(candidates)
}

async fn collect_flat_files_by_updated_at(
    root: &Path,
    scanned_files: &mut usize,
) -> io::Result<Vec<ThreadCandidate>> {
    let mut candidates = Vec::new();
    let mut dir = tokio::fs::read_dir(root).await?;
    while let Some(entry) = dir.next_entry().await? {
        if *scanned_files >= MAX_SCAN_FILES {
            break;
        }
        if !entry
            .file_type()
            .await
            .map(|ft| ft.is_file())
            .unwrap_or(false)
        {
            continue;
        }
        let file_name = entry.file_name();
        let Some(name_str) = file_name.to_str() else {
            continue;
        };
        if !name_str.starts_with("rollout-") || !name_str.ends_with(".jsonl") {
            continue;
        }
        let Some((_ts, id)) = parse_timestamp_uuid_from_filename(name_str) else {
            continue;
        };
        *scanned_files += 1;
        if *scanned_files > MAX_SCAN_FILES {
            break;
        }
        let updated_at = file_modified_time(&entry.path()).await.unwrap_or(None);
        candidates.push(ThreadCandidate {
            path: entry.path(),
            id,
            updated_at,
        });
    }

    Ok(candidates)
}

async fn walk_rollout_files(
    root: &Path,
    scanned_files: &mut usize,
    visitor: &mut impl RolloutFileVisitor,
) -> io::Result<()> {
    let year_dirs = collect_dirs_desc(root, |s| s.parse::<u16>().ok()).await?;

    'outer: for (_year, year_path) in year_dirs.iter() {
        if *scanned_files >= MAX_SCAN_FILES {
            break;
        }
        let month_dirs = collect_dirs_desc(year_path, |s| s.parse::<u8>().ok()).await?;
        for (_month, month_path) in month_dirs.iter() {
            if *scanned_files >= MAX_SCAN_FILES {
                break 'outer;
            }
            let day_dirs = collect_dirs_desc(month_path, |s| s.parse::<u8>().ok()).await?;
            for (_day, day_path) in day_dirs.iter() {
                if *scanned_files >= MAX_SCAN_FILES {
                    break 'outer;
                }
                let day_files = collect_rollout_day_files(day_path).await?;
                for (ts, id, path) in day_files.into_iter() {
                    *scanned_files += 1;
                    if *scanned_files > MAX_SCAN_FILES {
                        break 'outer;
                    }
                    if let ControlFlow::Break(()) =
                        visitor.visit(ts, id, path, *scanned_files).await
                    {
                        break 'outer;
                    }
                }
            }
        }
    }

    Ok(())
}

struct ProviderMatcher<'a> {
    filters: &'a [String],
    matches_default_provider: bool,
}

impl<'a> ProviderMatcher<'a> {
    fn new(filters: &'a [String], default_provider: &'a str) -> Option<Self> {
        if filters.is_empty() {
            return None;
        }

        let matches_default_provider = filters.iter().any(|provider| provider == default_provider);
        Some(Self {
            filters,
            matches_default_provider,
        })
    }

    fn matches(&self, session_provider: Option<&str>) -> bool {
        match session_provider {
            Some(provider) => self.filters.iter().any(|candidate| candidate == provider),
            None => self.matches_default_provider,
        }
    }
}

async fn read_head_summary(path: &Path, head_limit: usize) -> io::Result<HeadTailSummary> {
    use tokio::io::AsyncBufReadExt;

    let file = tokio::fs::File::open(path).await?;
    let reader = tokio::io::BufReader::new(file);
    let mut lines = reader.lines();
    let mut summary = HeadTailSummary::default();
    let mut lines_scanned = 0usize;

    while lines_scanned < head_limit
        || (summary.saw_session_meta
            && (summary.preview.is_none() || summary.first_user_message.is_none())
            && lines_scanned < head_limit + USER_EVENT_SCAN_LIMIT)
    {
        let line_opt = lines.next_line().await?;
        let Some(line) = line_opt else { break };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        lines_scanned += 1;

        let parsed: Result<RolloutLine, _> = serde_json::from_str(trimmed);
        let Ok(rollout_line) = parsed else { continue };

        match rollout_line.item {
            RolloutItem::SessionMeta(session_meta_line) => {
                if !summary.saw_session_meta {
                    summary.source = Some(session_meta_line.meta.source.clone());
                    summary.model_provider = session_meta_line.meta.model_provider.clone();
                    summary.thread_id = Some(session_meta_line.meta.id);
                    summary.cwd = Some(session_meta_line.meta.cwd.clone());
                    summary.git_branch = session_meta_line
                        .git
                        .as_ref()
                        .and_then(|git| git.branch.clone());
                    summary.git_sha = session_meta_line
                        .git
                        .as_ref()
                        .and_then(|git| git.commit_hash.as_ref().map(|sha| sha.0.clone()));
                    summary.git_origin_url = session_meta_line
                        .git
                        .as_ref()
                        .and_then(|git| git.repository_url.clone());
                    summary.cli_version = Some(session_meta_line.meta.cli_version);
                    summary.created_at = Some(session_meta_line.meta.timestamp.clone());
                    summary.saw_session_meta = true;
                }
            }
            RolloutItem::ResponseItem(item) => {
                summary.created_at = summary
                    .created_at
                    .clone()
                    .or_else(|| Some(rollout_line.timestamp.clone()));
                if let Some(preview) = response_item_preview(&item) {
                    if summary.preview.is_none() {
                        summary.preview = Some(preview.clone());
                    }
                    if summary.first_user_message.is_none() {
                        summary.first_user_message = Some(preview);
                    }
                }
            }
            RolloutItem::TurnContext(_) => {}
            RolloutItem::Compacted(_) => {}
            RolloutItem::EventMsg(_) => {}
        }

        if summary.saw_session_meta
            && summary.preview.is_some()
            && summary.first_user_message.is_some()
        {
            break;
        }
    }

    Ok(summary)
}

pub async fn read_head_for_summary(path: &Path) -> io::Result<Vec<serde_json::Value>> {
    use tokio::io::AsyncBufReadExt;

    let file = tokio::fs::File::open(path).await?;
    let reader = tokio::io::BufReader::new(file);
    let mut lines = reader.lines();
    let mut head = Vec::new();

    while head.len() < HEAD_RECORD_LIMIT {
        let Some(line) = lines.next_line().await? else {
            break;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(rollout_line) = serde_json::from_str::<RolloutLine>(trimmed) {
            match rollout_line.item {
                RolloutItem::SessionMeta(session_meta_line) => {
                    if let Ok(value) = serde_json::to_value(session_meta_line) {
                        head.push(value);
                    }
                }
                RolloutItem::ResponseItem(item) => {
                    if let Ok(value) = serde_json::to_value(item) {
                        head.push(value);
                    }
                }
                RolloutItem::Compacted(_)
                | RolloutItem::TurnContext(_)
                | RolloutItem::EventMsg(_) => {}
            }
        }
    }

    Ok(head)
}

fn strip_user_message_prefix(text: &str) -> &str {
    match text.find(USER_MESSAGE_BEGIN) {
        Some(idx) => text[idx + USER_MESSAGE_BEGIN.len()..].trim(),
        None => text.trim(),
    }
}

fn response_item_preview(item: &ResponseItem) -> Option<String> {
    let ResponseItem::Message { role, content, .. } = item else {
        return None;
    };
    if role != "user" {
        return None;
    }
    let mut text = String::new();
    let mut has_image = false;
    for item in content {
        match item {
            ContentItem::InputText { text: part } => text.push_str(part),
            ContentItem::InputImage { .. } => has_image = true,
            ContentItem::OutputText { .. } => {}
        }
    }
    let message = strip_user_message_prefix(text.as_str());
    if !message.is_empty() {
        Some(message.to_string())
    } else if has_image {
        Some("[Image]".to_string())
    } else {
        None
    }
}

pub async fn read_session_meta_line(path: &Path) -> io::Result<SessionMetaLine> {
    let head = read_head_for_summary(path).await?;
    let Some(first) = head.first() else {
        return Err(io::Error::other(format!(
            "rollout at {} is empty",
            path.display()
        )));
    };
    serde_json::from_value::<SessionMetaLine>(first.clone()).map_err(|_| {
        io::Error::other(format!(
            "rollout at {} does not start with session metadata",
            path.display()
        ))
    })
}

async fn file_modified_time(path: &Path) -> io::Result<Option<OffsetDateTime>> {
    let meta = tokio::fs::metadata(path).await?;
    let modified = meta.modified().ok();
    let Some(modified) = modified else {
        return Ok(None);
    };
    let dt = OffsetDateTime::from(modified);
    Ok(truncate_to_millis(dt))
}

fn format_rfc3339(dt: OffsetDateTime) -> Option<String> {
    dt.format(&Rfc3339).ok()
}

fn truncate_to_millis(dt: OffsetDateTime) -> Option<OffsetDateTime> {
    let millis_nanos = (dt.nanosecond() / 1_000_000) * 1_000_000;
    dt.replace_nanosecond(millis_nanos).ok()
}

async fn find_thread_path_by_id_str_in_subdir(
    codex_home: &Path,
    subdir: &str,
    id_str: &str,
    state_db_ctx: Option<&codex_state::StateRuntime>,
) -> io::Result<Option<PathBuf>> {
    if Uuid::parse_str(id_str).is_err() {
        return Ok(None);
    }

    let archived_only = match subdir {
        SESSIONS_SUBDIR => Some(false),
        ARCHIVED_SESSIONS_SUBDIR => Some(true),
        _ => None,
    };
    let thread_id = ThreadId::from_string(id_str).ok();
    let mut unverified_db_path = None;
    let mut fallback_reason = state_db_ctx.is_none().then_some("db_unavailable");
    if let Some(state_db_ctx) = state_db_ctx
        && let Some(thread_id) = thread_id
    {
        match state_db_ctx
            .find_rollout_path_by_id(thread_id, archived_only)
            .await
        {
            Ok(Some(db_path)) => {
                if tokio::fs::try_exists(&db_path).await.unwrap_or(false) {
                    match read_session_meta_line(&db_path).await {
                        Ok(meta_line) if meta_line.meta.id == thread_id => {
                            return Ok(Some(db_path));
                        }
                        Ok(meta_line) => {
                            tracing::error!(
                                "state db returned rollout path for thread {id_str} but file belongs to thread {}: {}",
                                meta_line.meta.id,
                                db_path.display()
                            );
                            tracing::warn!(
                                "state db discrepancy during find_thread_path_by_id_str_in_subdir: mismatched_db_path"
                            );
                            codex_state::record_fallback("find_thread_path", "mismatch", None);
                        }
                        Err(err) => {
                            tracing::debug!(
                                "state db returned rollout path for thread {id_str} that could not be verified: {}: {err}",
                                db_path.display()
                            );
                            unverified_db_path = Some(db_path);
                        }
                    }
                } else {
                    tracing::error!(
                        "state db returned stale rollout path for thread {id_str}: {}",
                        db_path.display()
                    );
                    tracing::warn!(
                        "state db discrepancy during find_thread_path_by_id_str_in_subdir: stale_db_path"
                    );
                    codex_state::record_fallback("find_thread_path", "stale_path", None);
                }
            }
            Ok(None) => fallback_reason = Some("missing_row"),
            Err(err) => {
                tracing::warn!(
                    "state db find_rollout_path_by_id failed during find_path_query: {err}"
                );
                fallback_reason = Some("db_error");
            }
        }
    }

    let mut root = codex_home.to_path_buf();
    root.push(subdir);
    if !root.exists() {
        return Ok(unverified_db_path);
    }
    let found = find_rollout_path_by_thread_id(root.as_path(), id_str, thread_id).await?;
    if let Some(found_path) = found.as_ref() {
        tracing::debug!("state db missing rollout path for thread {id_str}");
        tracing::warn!(
            "state db discrepancy during find_thread_path_by_id_str_in_subdir: falling_back"
        );
        if let Some(reason) = fallback_reason {
            codex_state::record_fallback("find_thread_path", reason, None);
        }
        state_db::read_repair_rollout_path(
            state_db_ctx,
            thread_id,
            archived_only,
            found_path.as_path(),
        )
        .await;
    }

    Ok(found.or(unverified_db_path))
}

async fn find_rollout_path_by_thread_id(
    root: &Path,
    id_str: &str,
    thread_id: Option<ThreadId>,
) -> io::Result<Option<PathBuf>> {
    let mut scanned_files = 0usize;
    let mut dirs = vec![root.to_path_buf()];

    while let Some(dir) = dirs.pop() {
        let mut entries = match tokio::fs::read_dir(dir).await {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err),
        };

        while let Some(entry) = entries.next_entry().await? {
            let file_type = entry.file_type().await?;
            let path = entry.path();
            if file_type.is_dir() {
                dirs.push(path);
                continue;
            }
            if !file_type.is_file()
                || path.extension().and_then(|extension| extension.to_str()) != Some("jsonl")
            {
                continue;
            }

            scanned_files += 1;
            if scanned_files > MAX_SCAN_FILES {
                return Ok(None);
            }

            let name_matches = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(parse_timestamp_uuid_from_filename)
                .is_some_and(|(_ts, id)| id.to_string() == id_str);
            if name_matches {
                return Ok(Some(path));
            }

            if let Some(thread_id) = thread_id
                && let Ok(meta_line) = read_session_meta_line(path.as_path()).await
                && meta_line.meta.id == thread_id
            {
                return Ok(Some(path));
            }
        }
    }

    Ok(None)
}

pub async fn find_thread_path_by_id_str(
    codex_home: &Path,
    id_str: &str,
    state_db_ctx: Option<&codex_state::StateRuntime>,
) -> io::Result<Option<PathBuf>> {
    find_thread_path_by_id_str_in_subdir(codex_home, SESSIONS_SUBDIR, id_str, state_db_ctx).await
}

pub async fn find_archived_thread_path_by_id_str(
    codex_home: &Path,
    id_str: &str,
    state_db_ctx: Option<&codex_state::StateRuntime>,
) -> io::Result<Option<PathBuf>> {
    find_thread_path_by_id_str_in_subdir(codex_home, ARCHIVED_SESSIONS_SUBDIR, id_str, state_db_ctx)
        .await
}

pub fn rollout_date_parts(file_name: &OsStr) -> Option<(String, String, String)> {
    let name = file_name.to_string_lossy();
    let date = name.strip_prefix("rollout-")?.get(..10)?;
    let year = date.get(..4)?.to_string();
    let month = date.get(5..7)?.to_string();
    let day = date.get(8..10)?.to_string();
    Some((year, month, day))
}
