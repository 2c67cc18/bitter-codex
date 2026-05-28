use super::*;
use crate::SortDirection;
use std::sync::atomic::Ordering;

impl StateRuntime {
    pub async fn get_thread(&self, id: ThreadId) -> anyhow::Result<Option<crate::ThreadMetadata>> {
        let row = sqlx::query(
            r#"
SELECT
    threads.id,
    threads.rollout_path,
    threads.created_at_ms AS created_at,
    threads.updated_at_ms AS updated_at,
    threads.source,
    threads.model_provider,
    threads.model,
    threads.reasoning_effort,
    threads.cwd,
    threads.cli_version,
    threads.title,
    threads.preview,
    threads.tokens_used,
    threads.first_user_message,
    threads.archived_at,
    threads.git_sha,
    threads.git_branch,
    threads.git_origin_url
FROM threads
WHERE threads.id = ?
            "#,
        )
        .bind(id.to_string())
        .fetch_optional(self.pool.as_ref())
        .await?;
        row.map(|row| ThreadRow::try_from_row(&row).and_then(ThreadMetadata::try_from))
            .transpose()
    }

    pub async fn set_thread_preview_if_empty(
        &self,
        thread_id: ThreadId,
        preview: &str,
    ) -> anyhow::Result<bool> {
        let preview = preview.trim();
        if preview.is_empty() {
            return Ok(false);
        }
        let result = sqlx::query(
            r#"
UPDATE threads
SET preview = ?
WHERE id = ? AND preview = ''
            "#,
        )
        .bind(preview)
        .bind(thread_id.to_string())
        .execute(self.pool.as_ref())
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn get_dynamic_tools(
        &self,
        thread_id: ThreadId,
    ) -> anyhow::Result<Option<Vec<DynamicToolSpec>>> {
        let rows = sqlx::query(
            r#"
SELECT namespace, name, description, input_schema, defer_loading
FROM thread_dynamic_tools
WHERE thread_id = ?
ORDER BY position ASC
            "#,
        )
        .bind(thread_id.to_string())
        .fetch_all(self.pool.as_ref())
        .await?;
        if rows.is_empty() {
            return Ok(None);
        }
        let mut tools = Vec::with_capacity(rows.len());
        for row in rows {
            let input_schema: String = row.try_get("input_schema")?;
            let input_schema = serde_json::from_str::<Value>(input_schema.as_str())?;
            tools.push(DynamicToolSpec {
                namespace: row.try_get("namespace")?,
                name: row.try_get("name")?,
                description: row.try_get("description")?,
                input_schema,
                defer_loading: row.try_get("defer_loading")?,
            });
        }
        Ok(Some(tools))
    }

    pub async fn find_rollout_path_by_id(
        &self,
        id: ThreadId,
        archived_only: Option<bool>,
    ) -> anyhow::Result<Option<PathBuf>> {
        let mut builder =
            QueryBuilder::<Sqlite>::new("SELECT rollout_path FROM threads WHERE id = ");
        builder.push_bind(id.to_string());
        match archived_only {
            Some(true) => {
                builder.push(" AND archived = 1");
            }
            Some(false) => {
                builder.push(" AND archived = 0");
            }
            None => {}
        }
        let row = builder.build().fetch_optional(self.pool.as_ref()).await?;
        Ok(row
            .and_then(|r| r.try_get::<String, _>("rollout_path").ok())
            .map(PathBuf::from))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn find_thread_by_exact_title(
        &self,
        title: &str,
        allowed_sources: &[String],
        model_providers: Option<&[String]>,
        archived_only: bool,
        cwd: Option<&Path>,
    ) -> anyhow::Result<Option<crate::ThreadMetadata>> {
        let mut builder = QueryBuilder::<Sqlite>::new("");
        push_thread_select_columns(&mut builder);
        builder.push(" FROM threads");
        push_thread_filters(
            &mut builder,
            ThreadFilterOptions {
                archived_only,
                allowed_sources,
                model_providers,
                cwd_filters: None,
                anchor: None,
                sort_key: crate::SortKey::UpdatedAt,
                sort_direction: SortDirection::Desc,
                search_term: None,
            },
        );
        builder.push(" AND threads.title = ");
        builder.push_bind(title);
        if let Some(cwd) = cwd {
            builder.push(" AND threads.cwd = ");
            builder.push_bind(cwd.display().to_string());
        }
        push_thread_order_and_limit(
            &mut builder,
            crate::SortKey::UpdatedAt,
            SortDirection::Desc,
            1,
        );

        let row = builder.build().fetch_optional(self.pool.as_ref()).await?;
        row.map(|row| ThreadRow::try_from_row(&row).and_then(crate::ThreadMetadata::try_from))
            .transpose()
    }

    pub async fn list_threads(
        &self,
        page_size: usize,
        filters: ThreadFilterOptions<'_>,
    ) -> anyhow::Result<crate::ThreadsPage> {
        let limit = page_size.saturating_add(1);
        let sort_key = filters.sort_key;
        let sort_direction = filters.sort_direction;

        let mut builder = QueryBuilder::<Sqlite>::new("");
        push_thread_select_columns(&mut builder);
        builder.push(" FROM threads");
        push_thread_filters(&mut builder, filters);
        push_thread_order_and_limit(&mut builder, sort_key, sort_direction, limit);

        let rows = builder.build().fetch_all(self.pool.as_ref()).await?;
        let mut items = rows
            .into_iter()
            .map(|row| ThreadRow::try_from_row(&row).and_then(ThreadMetadata::try_from))
            .collect::<Result<Vec<_>, _>>()?;
        let num_scanned_rows = items.len();
        let next_anchor = if items.len() > page_size {
            items.pop();
            items
                .last()
                .and_then(|item| anchor_from_item(item, sort_key))
        } else {
            None
        };
        Ok(ThreadsPage {
            items,
            next_anchor,
            num_scanned_rows,
        })
    }

    pub async fn list_thread_ids(
        &self,
        limit: usize,
        anchor: Option<&crate::Anchor>,
        sort_key: crate::SortKey,
        allowed_sources: &[String],
        model_providers: Option<&[String]>,
        archived_only: bool,
    ) -> anyhow::Result<Vec<ThreadId>> {
        let mut builder = QueryBuilder::<Sqlite>::new("SELECT threads.id FROM threads");
        push_thread_filters(
            &mut builder,
            ThreadFilterOptions {
                archived_only,
                allowed_sources,
                model_providers,
                cwd_filters: None,
                anchor,
                sort_key,
                sort_direction: SortDirection::Desc,
                search_term: None,
            },
        );
        push_thread_order_and_limit(&mut builder, sort_key, SortDirection::Desc, limit);

        let rows = builder.build().fetch_all(self.pool.as_ref()).await?;
        rows.into_iter()
            .map(|row| {
                let id: String = row.try_get("id")?;
                Ok(ThreadId::try_from(id)?)
            })
            .collect()
    }

    pub async fn insert_thread_if_absent(
        &self,
        metadata: &crate::ThreadMetadata,
    ) -> anyhow::Result<bool> {
        let updated_at = self.allocate_thread_updated_at(metadata.updated_at)?;
        let preview = metadata_preview(metadata);
        let result = sqlx::query(
            r#"
INSERT INTO threads (
    id,
    rollout_path,
    created_at,
    updated_at,
    created_at_ms,
    updated_at_ms,
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
    archived,
    archived_at,
    git_sha,
    git_branch,
    git_origin_url
) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
ON CONFLICT(id) DO NOTHING
            "#,
        )
        .bind(metadata.id.to_string())
        .bind(metadata.rollout_path.display().to_string())
        .bind(datetime_to_epoch_seconds(metadata.created_at))
        .bind(datetime_to_epoch_seconds(updated_at))
        .bind(datetime_to_epoch_millis(metadata.created_at))
        .bind(datetime_to_epoch_millis(updated_at))
        .bind(metadata.source.as_str())
        .bind(metadata.model_provider.as_str())
        .bind(metadata.model.as_deref())
        .bind(
            metadata
                .reasoning_effort
                .as_ref()
                .map(crate::extract::enum_to_string),
        )
        .bind(metadata.cwd.display().to_string())
        .bind(metadata.cli_version.as_str())
        .bind(metadata.title.as_str())
        .bind(preview)
        .bind(metadata.tokens_used)
        .bind(metadata.first_user_message.as_deref().unwrap_or_default())
        .bind(metadata.archived_at.is_some())
        .bind(metadata.archived_at.map(datetime_to_epoch_seconds))
        .bind(metadata.git_sha.as_deref())
        .bind(metadata.git_branch.as_deref())
        .bind(metadata.git_origin_url.as_deref())
        .execute(self.pool.as_ref())
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn update_thread_title(
        &self,
        thread_id: ThreadId,
        title: &str,
    ) -> anyhow::Result<bool> {
        let result = sqlx::query("UPDATE threads SET title = ? WHERE id = ?")
            .bind(title)
            .bind(thread_id.to_string())
            .execute(self.pool.as_ref())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn touch_thread_updated_at(
        &self,
        thread_id: ThreadId,
        updated_at: DateTime<Utc>,
    ) -> anyhow::Result<bool> {
        let updated_at = self.allocate_thread_updated_at(updated_at)?;
        let result =
            sqlx::query("UPDATE threads SET updated_at = ?, updated_at_ms = ? WHERE id = ?")
                .bind(datetime_to_epoch_seconds(updated_at))
                .bind(datetime_to_epoch_millis(updated_at))
                .bind(thread_id.to_string())
                .execute(self.pool.as_ref())
                .await?;
        Ok(result.rows_affected() > 0)
    }

    fn allocate_thread_updated_at(
        &self,
        updated_at: DateTime<Utc>,
    ) -> anyhow::Result<DateTime<Utc>> {
        let candidate = datetime_to_epoch_millis(updated_at);
        let allocated = loop {
            let current = self.thread_updated_at_millis.load(Ordering::Relaxed);

            if candidate > current {
                if self
                    .thread_updated_at_millis
                    .compare_exchange(current, candidate, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
                {
                    break candidate;
                }
                continue;
            }

            if candidate.saturating_add(1000) <= current {
                break candidate;
            }

            let bumped = current.saturating_add(1);
            if self
                .thread_updated_at_millis
                .compare_exchange(current, bumped, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                break bumped;
            }
        };
        epoch_millis_to_datetime(allocated)
    }

    pub async fn update_thread_git_info(
        &self,
        thread_id: ThreadId,
        git_sha: Option<Option<&str>>,
        git_branch: Option<Option<&str>>,
        git_origin_url: Option<Option<&str>>,
    ) -> anyhow::Result<bool> {
        let result = sqlx::query(
            r#"
UPDATE threads
SET
    git_sha = CASE WHEN ? THEN ? ELSE git_sha END,
    git_branch = CASE WHEN ? THEN ? ELSE git_branch END,
    git_origin_url = CASE WHEN ? THEN ? ELSE git_origin_url END
WHERE id = ?
            "#,
        )
        .bind(git_sha.is_some())
        .bind(git_sha.flatten())
        .bind(git_branch.is_some())
        .bind(git_branch.flatten())
        .bind(git_origin_url.is_some())
        .bind(git_origin_url.flatten())
        .bind(thread_id.to_string())
        .execute(self.pool.as_ref())
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn upsert_thread(&self, metadata: &crate::ThreadMetadata) -> anyhow::Result<()> {
        let updated_at = self.allocate_thread_updated_at(metadata.updated_at)?;
        let preview = metadata_preview(metadata);

        sqlx::query(
            r#"
INSERT INTO threads (
    id,
    rollout_path,
    created_at,
    updated_at,
    created_at_ms,
    updated_at_ms,
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
    archived,
    archived_at,
    git_sha,
    git_branch,
    git_origin_url
) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
ON CONFLICT(id) DO UPDATE SET
    rollout_path = excluded.rollout_path,
    created_at = excluded.created_at,
    updated_at = excluded.updated_at,
    created_at_ms = excluded.created_at_ms,
    updated_at_ms = excluded.updated_at_ms,
    source = excluded.source,
    model_provider = excluded.model_provider,
    model = excluded.model,
    reasoning_effort = excluded.reasoning_effort,
    cwd = excluded.cwd,
    cli_version = excluded.cli_version,
    title = excluded.title,
    preview = COALESCE(NULLIF(excluded.preview, ''), threads.preview),
    tokens_used = excluded.tokens_used,
    first_user_message = excluded.first_user_message,
    archived = excluded.archived,
    archived_at = excluded.archived_at,
    git_sha = COALESCE(threads.git_sha, excluded.git_sha),
    git_branch = COALESCE(threads.git_branch, excluded.git_branch),
    git_origin_url = COALESCE(threads.git_origin_url, excluded.git_origin_url)
            "#,
        )
        .bind(metadata.id.to_string())
        .bind(metadata.rollout_path.display().to_string())
        .bind(datetime_to_epoch_seconds(metadata.created_at))
        .bind(datetime_to_epoch_seconds(updated_at))
        .bind(datetime_to_epoch_millis(metadata.created_at))
        .bind(datetime_to_epoch_millis(updated_at))
        .bind(metadata.source.as_str())
        .bind(metadata.model_provider.as_str())
        .bind(metadata.model.as_deref())
        .bind(
            metadata
                .reasoning_effort
                .as_ref()
                .map(crate::extract::enum_to_string),
        )
        .bind(metadata.cwd.display().to_string())
        .bind(metadata.cli_version.as_str())
        .bind(metadata.title.as_str())
        .bind(preview)
        .bind(metadata.tokens_used)
        .bind(metadata.first_user_message.as_deref().unwrap_or_default())
        .bind(metadata.archived_at.is_some())
        .bind(metadata.archived_at.map(datetime_to_epoch_seconds))
        .bind(metadata.git_sha.as_deref())
        .bind(metadata.git_branch.as_deref())
        .bind(metadata.git_origin_url.as_deref())
        .execute(self.pool.as_ref())
        .await?;
        Ok(())
    }

    pub async fn persist_dynamic_tools(
        &self,
        thread_id: ThreadId,
        tools: Option<&[DynamicToolSpec]>,
    ) -> anyhow::Result<()> {
        let Some(tools) = tools else {
            return Ok(());
        };
        if tools.is_empty() {
            return Ok(());
        }
        let thread_id = thread_id.to_string();
        let mut tx = self.pool.begin().await?;
        for (idx, tool) in tools.iter().enumerate() {
            let position = i64::try_from(idx).unwrap_or(i64::MAX);
            let input_schema = serde_json::to_string(&tool.input_schema)?;
            sqlx::query(
                r#"
INSERT INTO thread_dynamic_tools (
    thread_id,
    position,
    namespace,
    name,
    description,
    input_schema,
    defer_loading
) VALUES (?, ?, ?, ?, ?, ?, ?)
ON CONFLICT(thread_id, position) DO NOTHING
                "#,
            )
            .bind(thread_id.as_str())
            .bind(position)
            .bind(tool.namespace.as_deref())
            .bind(tool.name.as_str())
            .bind(tool.description.as_str())
            .bind(input_schema)
            .bind(tool.defer_loading)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn apply_rollout_items(
        &self,
        builder: &ThreadMetadataBuilder,
        items: &[RolloutItem],
        updated_at_override: Option<DateTime<Utc>>,
    ) -> anyhow::Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        let existing_metadata = self.get_thread(builder.id).await?;
        let mut metadata = existing_metadata
            .clone()
            .unwrap_or_else(|| builder.build(&self.default_provider));
        metadata.rollout_path = builder.rollout_path.clone();
        for item in items {
            apply_rollout_item(&mut metadata, item, &self.default_provider);
        }
        if let Some(existing_metadata) = existing_metadata.as_ref() {
            metadata.prefer_existing_git_info(existing_metadata);
        }
        let updated_at = match updated_at_override {
            Some(updated_at) => Some(updated_at),
            None => file_modified_time_utc(builder.rollout_path.as_path()).await,
        };
        if let Some(updated_at) = updated_at {
            metadata.updated_at = updated_at;
        }

        self.upsert_thread(&metadata).await?;
        let dynamic_tools = extract_dynamic_tools(items);
        if let Some(dynamic_tools) = dynamic_tools
            && let Err(err) = self
                .persist_dynamic_tools(builder.id, dynamic_tools.as_deref())
                .await
        {
            return Err(err);
        }
        Ok(())
    }

    pub async fn mark_archived(
        &self,
        thread_id: ThreadId,
        rollout_path: &Path,
        archived_at: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        let Some(mut metadata) = self.get_thread(thread_id).await? else {
            return Ok(());
        };
        metadata.archived_at = Some(archived_at);
        metadata.rollout_path = rollout_path.to_path_buf();
        if let Some(updated_at) = file_modified_time_utc(rollout_path).await {
            metadata.updated_at = updated_at;
        }
        if metadata.id != thread_id {
            warn!(
                "thread id mismatch during archive: expected {thread_id}, got {}",
                metadata.id
            );
        }
        self.upsert_thread(&metadata).await
    }

    pub async fn mark_unarchived(
        &self,
        thread_id: ThreadId,
        rollout_path: &Path,
    ) -> anyhow::Result<()> {
        let Some(mut metadata) = self.get_thread(thread_id).await? else {
            return Ok(());
        };
        metadata.archived_at = None;
        metadata.rollout_path = rollout_path.to_path_buf();
        if let Some(updated_at) = file_modified_time_utc(rollout_path).await {
            metadata.updated_at = updated_at;
        }
        if metadata.id != thread_id {
            warn!(
                "thread id mismatch during unarchive: expected {thread_id}, got {}",
                metadata.id
            );
        }
        self.upsert_thread(&metadata).await
    }

    pub async fn delete_thread(&self, thread_id: ThreadId) -> anyhow::Result<u64> {
        let result = sqlx::query("DELETE FROM threads WHERE id = ?")
            .bind(thread_id.to_string())
            .execute(self.pool.as_ref())
            .await?;
        let rows_affected = result.rows_affected();
        Ok(rows_affected)
    }
}

fn one_thread_id_from_rows(rows: Vec<sqlx::sqlite::SqliteRow>) -> anyhow::Result<Option<ThreadId>> {
    let mut ids = rows
        .into_iter()
        .map(|row| {
            let id: String = row.try_get("id")?;
            ThreadId::try_from(id).map_err(anyhow::Error::from)
        })
        .collect::<Result<Vec<_>, _>>()?;
    match ids.len() {
        0 => Ok(None),
        1 => Ok(ids.pop()),
        _ => Err(anyhow::anyhow!("multiple threads found")),
    }
}

pub(super) fn push_thread_select_columns(builder: &mut QueryBuilder<'_, Sqlite>) {
    builder.push(
        r#"
SELECT
    threads.id,
    threads.rollout_path,
    threads.created_at_ms AS created_at,
    threads.updated_at_ms AS updated_at,
    threads.source,
    threads.model_provider,
    threads.model,
    threads.reasoning_effort,
    threads.cwd,
    threads.cli_version,
    threads.title,
    threads.preview,
    threads.tokens_used,
    threads.first_user_message,
    threads.archived_at,
    threads.git_sha,
    threads.git_branch,
    threads.git_origin_url
"#,
    );
}

pub(super) fn extract_dynamic_tools(items: &[RolloutItem]) -> Option<Option<Vec<DynamicToolSpec>>> {
    items.iter().find_map(|item| match item {
        RolloutItem::SessionMeta(meta_line) => Some(meta_line.meta.dynamic_tools.clone()),
        RolloutItem::ResponseItem(_)
        | RolloutItem::Compacted(_)
        | RolloutItem::TurnContext(_)
        | RolloutItem::EventMsg(_) => None,
    })
}

#[derive(Clone, Copy)]
pub struct ThreadFilterOptions<'a> {
    pub archived_only: bool,
    pub allowed_sources: &'a [String],
    pub model_providers: Option<&'a [String]>,
    pub cwd_filters: Option<&'a [PathBuf]>,
    pub anchor: Option<&'a crate::Anchor>,
    pub sort_key: SortKey,
    pub sort_direction: SortDirection,
    pub search_term: Option<&'a str>,
}

pub(super) fn push_thread_filters<'a>(
    builder: &mut QueryBuilder<'a, Sqlite>,
    options: ThreadFilterOptions<'a>,
) {
    let ThreadFilterOptions {
        archived_only,
        allowed_sources,
        model_providers,
        cwd_filters,
        anchor,
        sort_key,
        sort_direction,
        search_term,
    } = options;
    builder.push(" WHERE 1 = 1");
    if archived_only {
        builder.push(" AND threads.archived = 1");
    } else {
        builder.push(" AND threads.archived = 0");
    }
    builder.push(" AND threads.preview <> ''");
    if !allowed_sources.is_empty() {
        builder.push(" AND threads.source IN (");
        let mut separated = builder.separated(", ");
        for source in allowed_sources {
            separated.push_bind(source);
        }
        separated.push_unseparated(")");
    }
    if let Some(model_providers) = model_providers
        && !model_providers.is_empty()
    {
        builder.push(" AND threads.model_provider IN (");
        let mut separated = builder.separated(", ");
        for provider in model_providers {
            separated.push_bind(provider);
        }
        separated.push_unseparated(")");
    }
    match cwd_filters {
        Some([]) => {
            builder.push(" AND 1 = 0");
        }
        Some(cwd_filters) => {
            builder.push(" AND threads.cwd IN (");
            let mut separated = builder.separated(", ");
            for cwd in cwd_filters {
                separated.push_bind(cwd.display().to_string());
            }
            separated.push_unseparated(")");
        }
        None => {}
    }
    if let Some(search_term) = search_term {
        builder.push(" AND (instr(threads.title, ");
        builder.push_bind(search_term);
        builder.push(") > 0 OR instr(threads.preview, ");
        builder.push_bind(search_term);
        builder.push(") > 0)");
    }
    if let Some(anchor) = anchor {
        let anchor_ts = datetime_to_epoch_millis(anchor.ts);
        let column = match sort_key {
            SortKey::CreatedAt => "threads.created_at_ms",
            SortKey::UpdatedAt => "threads.updated_at_ms",
        };
        let operator = match sort_direction {
            SortDirection::Asc => ">",
            SortDirection::Desc => "<",
        };
        builder.push(" AND (");
        builder.push(column);
        builder.push(" ");
        builder.push(operator);
        builder.push(" ");
        builder.push_bind(anchor_ts);
        builder.push(")");
    }
}

pub(super) fn push_thread_order_and_limit(
    builder: &mut QueryBuilder<'_, Sqlite>,
    sort_key: SortKey,
    sort_direction: SortDirection,
    limit: usize,
) {
    let order_column = match sort_key {
        SortKey::CreatedAt => "threads.created_at_ms",
        SortKey::UpdatedAt => "threads.updated_at_ms",
    };
    let order_direction = match sort_direction {
        SortDirection::Asc => "ASC",
        SortDirection::Desc => "DESC",
    };
    builder.push(" ORDER BY ");
    builder.push(order_column);
    builder.push(" ");
    builder.push(order_direction);
    builder.push(" LIMIT ");
    builder.push_bind(limit as i64);
}

fn metadata_preview(metadata: &crate::ThreadMetadata) -> &str {
    metadata
        .preview
        .as_deref()
        .or(metadata.first_user_message.as_deref())
        .unwrap_or_default()
}
