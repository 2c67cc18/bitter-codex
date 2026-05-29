use super::*;

const LOG_RETENTION_DAYS: i64 = 10;

impl StateRuntime {
    pub async fn insert_log(&self, entry: &LogEntry) -> anyhow::Result<()> {
        self.insert_logs(std::slice::from_ref(entry)).await
    }

    pub async fn insert_logs(&self, entries: &[LogEntry]) -> anyhow::Result<()> {
        if entries.is_empty() {
            return Ok(());
        }

        let mut tx = self.logs_pool.begin().await?;
        let mut builder = QueryBuilder::<Sqlite>::new(
            "INSERT INTO logs (ts, ts_nanos, level, target, message, thread_id, process_uuid, module_path, file, line, estimated_bytes) ",
        );
        builder.push_values(entries, |mut row, entry| {
            let estimated_bytes = entry.message.as_ref().map_or(0, String::len) as i64
                + entry.level.len() as i64
                + entry.target.len() as i64
                + entry.module_path.as_ref().map_or(0, String::len) as i64
                + entry.file.as_ref().map_or(0, String::len) as i64;
            row.push_bind(entry.ts)
                .push_bind(entry.ts_nanos)
                .push_bind(&entry.level)
                .push_bind(&entry.target)
                .push_bind(&entry.message)
                .push_bind(&entry.thread_id)
                .push_bind(&entry.process_uuid)
                .push_bind(&entry.module_path)
                .push_bind(&entry.file)
                .push_bind(entry.line)
                .push_bind(estimated_bytes);
        });
        builder.build().execute(&mut *tx).await?;
        self.prune_logs_after_insert(entries, &mut tx).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn prune_logs_after_insert(
        &self,
        entries: &[LogEntry],
        tx: &mut SqliteConnection,
    ) -> anyhow::Result<()> {
        let thread_ids: BTreeSet<&str> = entries
            .iter()
            .filter_map(|entry| entry.thread_id.as_deref())
            .collect();
        if !thread_ids.is_empty() {
            let mut over_limit_threads_query =
                QueryBuilder::<Sqlite>::new("SELECT thread_id FROM logs WHERE thread_id IN (");
            {
                let mut separated = over_limit_threads_query.separated(", ");
                for thread_id in &thread_ids {
                    separated.push_bind(*thread_id);
                }
            }
            over_limit_threads_query.push(") GROUP BY thread_id HAVING SUM(");
            over_limit_threads_query.push("estimated_bytes");
            over_limit_threads_query.push(") > ");
            over_limit_threads_query.push_bind(LOG_PARTITION_SIZE_LIMIT_BYTES);
            over_limit_threads_query.push(" OR COUNT(*) > ");
            over_limit_threads_query.push_bind(LOG_PARTITION_ROW_LIMIT);
            let over_limit_thread_ids: Vec<String> = over_limit_threads_query
                .build()
                .fetch_all(&mut *tx)
                .await?
                .into_iter()
                .map(|row| row.try_get("thread_id"))
                .collect::<Result<_, _>>()?;
            if !over_limit_thread_ids.is_empty() {
                let mut prune_threads = QueryBuilder::<Sqlite>::new(
                    r#"
DELETE FROM logs
WHERE id IN (
    SELECT id
    FROM (
        SELECT
            id,
            SUM(
"#,
                );
                prune_threads.push("estimated_bytes");
                prune_threads.push(
                    r#"
            ) OVER (
                PARTITION BY thread_id
                ORDER BY ts DESC, ts_nanos DESC, id DESC
            ) AS cumulative_bytes,
            ROW_NUMBER() OVER (
                PARTITION BY thread_id
                ORDER BY ts DESC, ts_nanos DESC, id DESC
            ) AS row_number
        FROM logs
        WHERE thread_id IN (
"#,
                );
                {
                    let mut separated = prune_threads.separated(", ");
                    for thread_id in &over_limit_thread_ids {
                        separated.push_bind(thread_id);
                    }
                }
                prune_threads.push(
                    r#"
        )
    )
    WHERE cumulative_bytes >
"#,
                );
                prune_threads.push_bind(LOG_PARTITION_SIZE_LIMIT_BYTES);
                prune_threads.push(" OR row_number > ");
                prune_threads.push_bind(LOG_PARTITION_ROW_LIMIT);
                prune_threads.push("\n)");
                prune_threads.build().execute(&mut *tx).await?;
            }
        }

        let threadless_process_uuids: BTreeSet<&str> = entries
            .iter()
            .filter(|entry| entry.thread_id.is_none())
            .filter_map(|entry| entry.process_uuid.as_deref())
            .collect();
        let has_threadless_null_process_uuid = entries
            .iter()
            .any(|entry| entry.thread_id.is_none() && entry.process_uuid.is_none());
        if !threadless_process_uuids.is_empty() {
            let mut over_limit_processes_query = QueryBuilder::<Sqlite>::new(
                "SELECT process_uuid FROM logs WHERE thread_id IS NULL AND process_uuid IN (",
            );
            {
                let mut separated = over_limit_processes_query.separated(", ");
                for process_uuid in &threadless_process_uuids {
                    separated.push_bind(*process_uuid);
                }
            }
            over_limit_processes_query.push(") GROUP BY process_uuid HAVING SUM(");
            over_limit_processes_query.push("estimated_bytes");
            over_limit_processes_query.push(") > ");
            over_limit_processes_query.push_bind(LOG_PARTITION_SIZE_LIMIT_BYTES);
            over_limit_processes_query.push(" OR COUNT(*) > ");
            over_limit_processes_query.push_bind(LOG_PARTITION_ROW_LIMIT);
            let over_limit_process_uuids: Vec<String> = over_limit_processes_query
                .build()
                .fetch_all(&mut *tx)
                .await?
                .into_iter()
                .map(|row| row.try_get("process_uuid"))
                .collect::<Result<_, _>>()?;
            if !over_limit_process_uuids.is_empty() {
                let mut prune_threadless_process_logs = QueryBuilder::<Sqlite>::new(
                    r#"
DELETE FROM logs
WHERE id IN (
    SELECT id
    FROM (
        SELECT
            id,
            SUM(
"#,
                );
                prune_threadless_process_logs.push("estimated_bytes");
                prune_threadless_process_logs.push(
                    r#"
            ) OVER (
                PARTITION BY process_uuid
                ORDER BY ts DESC, ts_nanos DESC, id DESC
            ) AS cumulative_bytes,
            ROW_NUMBER() OVER (
                PARTITION BY process_uuid
                ORDER BY ts DESC, ts_nanos DESC, id DESC
            ) AS row_number
        FROM logs
        WHERE thread_id IS NULL
          AND process_uuid IN (
"#,
                );
                {
                    let mut separated = prune_threadless_process_logs.separated(", ");
                    for process_uuid in &over_limit_process_uuids {
                        separated.push_bind(process_uuid);
                    }
                }
                prune_threadless_process_logs.push(
                    r#"
          )
    )
    WHERE cumulative_bytes >
"#,
                );
                prune_threadless_process_logs.push_bind(LOG_PARTITION_SIZE_LIMIT_BYTES);
                prune_threadless_process_logs.push(" OR row_number > ");
                prune_threadless_process_logs.push_bind(LOG_PARTITION_ROW_LIMIT);
                prune_threadless_process_logs.push("\n)");
                prune_threadless_process_logs
                    .build()
                    .execute(&mut *tx)
                    .await?;
            }
        }
        if has_threadless_null_process_uuid {
            let mut null_process_usage_query = QueryBuilder::<Sqlite>::new("SELECT SUM(");
            null_process_usage_query.push("estimated_bytes");
            null_process_usage_query.push(
                ") AS total_bytes, COUNT(*) AS row_count FROM logs WHERE thread_id IS NULL AND process_uuid IS NULL",
            );
            let null_process_usage = null_process_usage_query.build().fetch_one(&mut *tx).await?;
            let total_null_process_bytes: Option<i64> =
                null_process_usage.try_get("total_bytes")?;
            let null_process_row_count: i64 = null_process_usage.try_get("row_count")?;

            if total_null_process_bytes.unwrap_or(0) > LOG_PARTITION_SIZE_LIMIT_BYTES
                || null_process_row_count > LOG_PARTITION_ROW_LIMIT
            {
                let mut prune_threadless_null_process_logs = QueryBuilder::<Sqlite>::new(
                    r#"
DELETE FROM logs
WHERE id IN (
    SELECT id
    FROM (
        SELECT
            id,
            SUM(
"#,
                );
                prune_threadless_null_process_logs.push("estimated_bytes");
                prune_threadless_null_process_logs.push(
                    r#"
            ) OVER (
                PARTITION BY process_uuid
                ORDER BY ts DESC, ts_nanos DESC, id DESC
            ) AS cumulative_bytes,
            ROW_NUMBER() OVER (
                PARTITION BY process_uuid
                ORDER BY ts DESC, ts_nanos DESC, id DESC
            ) AS row_number
        FROM logs
        WHERE thread_id IS NULL
          AND process_uuid IS NULL
    )
    WHERE cumulative_bytes >
"#,
                );
                prune_threadless_null_process_logs.push_bind(LOG_PARTITION_SIZE_LIMIT_BYTES);
                prune_threadless_null_process_logs.push(" OR row_number > ");
                prune_threadless_null_process_logs.push_bind(LOG_PARTITION_ROW_LIMIT);
                prune_threadless_null_process_logs.push("\n)");
                prune_threadless_null_process_logs
                    .build()
                    .execute(&mut *tx)
                    .await?;
            }
        }
        Ok(())
    }

    pub(crate) async fn delete_logs_before(&self, cutoff_ts: i64) -> anyhow::Result<u64> {
        let result = sqlx::query("DELETE FROM logs WHERE ts < ?")
            .bind(cutoff_ts)
            .execute(self.logs_pool.as_ref())
            .await?;
        Ok(result.rows_affected())
    }

    pub(crate) async fn run_logs_startup_maintenance(&self) -> anyhow::Result<()> {
        let Some(cutoff) =
            Utc::now().checked_sub_signed(chrono::Duration::days(LOG_RETENTION_DAYS))
        else {
            return Ok(());
        };
        self.delete_logs_before(cutoff.timestamp()).await?;

        sqlx::query("PRAGMA wal_checkpoint(PASSIVE)")
            .execute(self.logs_pool.as_ref())
            .await?;
        Ok(())
    }

    pub async fn query_logs(&self, query: &LogQuery) -> anyhow::Result<Vec<LogRow>> {
        let mut builder = QueryBuilder::<Sqlite>::new(
            "SELECT id, ts, ts_nanos, level, target, message, thread_id, process_uuid, file, line FROM logs WHERE 1 = 1",
        );
        push_log_filters(&mut builder, query);
        if query.descending {
            builder.push(" ORDER BY id DESC");
        } else {
            builder.push(" ORDER BY id ASC");
        }
        if let Some(limit) = query.limit {
            builder.push(" LIMIT ").push_bind(limit as i64);
        }

        let rows = builder
            .build_query_as::<LogRow>()
            .fetch_all(self.logs_pool.as_ref())
            .await?;
        Ok(rows)
    }

    pub async fn max_log_id(&self, query: &LogQuery) -> anyhow::Result<i64> {
        let mut builder =
            QueryBuilder::<Sqlite>::new("SELECT MAX(id) AS max_id FROM logs WHERE 1 = 1");
        push_log_filters(&mut builder, query);
        let row = builder.build().fetch_one(self.logs_pool.as_ref()).await?;
        let max_id: Option<i64> = row.try_get("max_id")?;
        Ok(max_id.unwrap_or(0))
    }
}

fn push_log_filters<'a>(builder: &mut QueryBuilder<'a, Sqlite>, query: &'a LogQuery) {
    if !query.levels_upper.is_empty() {
        builder.push(" AND UPPER(level) IN (");
        {
            let mut separated = builder.separated(", ");
            for level_upper in &query.levels_upper {
                separated.push_bind(level_upper.as_str());
            }
        }
        builder.push(")");
    }
    if let Some(from_ts) = query.from_ts {
        builder.push(" AND ts >= ").push_bind(from_ts);
    }
    if let Some(to_ts) = query.to_ts {
        builder.push(" AND ts <= ").push_bind(to_ts);
    }
    push_like_filters(builder, "module_path", &query.module_like);
    push_like_filters(builder, "file", &query.file_like);
    let has_thread_filter = !query.thread_ids.is_empty() || query.include_threadless;
    if has_thread_filter {
        builder.push(" AND (");
        let mut needs_or = false;
        for thread_id in &query.thread_ids {
            if needs_or {
                builder.push(" OR ");
            }
            builder.push("thread_id = ").push_bind(thread_id.as_str());
            needs_or = true;
        }
        if query.include_threadless {
            if needs_or {
                builder.push(" OR ");
            }
            builder.push("thread_id IS NULL");
        }
        builder.push(")");
    }
    if let Some(after_id) = query.after_id {
        builder.push(" AND id > ").push_bind(after_id);
    }
    if let Some(search) = query.search.as_ref() {
        builder.push(" AND INSTR(COALESCE(message, ''), ");
        builder.push_bind(search.as_str());
        builder.push(") > 0");
    }
}

fn push_like_filters<'a>(
    builder: &mut QueryBuilder<'a, Sqlite>,
    column: &str,
    filters: &'a [String],
) {
    if filters.is_empty() {
        return;
    }
    builder.push(" AND (");
    for (idx, filter) in filters.iter().enumerate() {
        if idx > 0 {
            builder.push(" OR ");
        }
        builder
            .push(column)
            .push(" LIKE '%' || ")
            .push_bind(filter.as_str())
            .push(" || '%'");
    }
    builder.push(")");
}

#[cfg(test)]
mod tests {
    use super::StateRuntime;
    use super::test_support::unique_temp_dir;
    use crate::LogEntry;
    use crate::LogQuery;
    use crate::logs_db_path;
    use pretty_assertions::assert_eq;
    use sqlx::SqlitePool;
    use sqlx::sqlite::SqliteConnectOptions;
    use std::path::Path;

    async fn open_db_pool(path: &Path) -> SqlitePool {
        SqlitePool::connect_with(
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(false),
        )
        .await
        .expect("open sqlite pool")
    }

    async fn log_row_count(path: &Path) -> i64 {
        let pool = open_db_pool(path).await;
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM logs")
            .fetch_one(&pool)
            .await
            .expect("count log rows");
        pool.close().await;
        count
    }

    #[tokio::test]
    async fn insert_logs_use_dedicated_log_database() {
        let codex_home = unique_temp_dir();
        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
            .await
            .expect("initialize runtime");

        runtime
            .insert_logs(&[LogEntry {
                ts: 1,
                ts_nanos: 0,
                level: "INFO".to_string(),
                target: "cli".to_string(),
                message: Some("dedicated-log-db".to_string()),
                thread_id: Some("thread-1".to_string()),
                process_uuid: Some("proc-1".to_string()),
                module_path: Some("mod".to_string()),
                file: Some("main.rs".to_string()),
                line: Some(7),
            }])
            .await
            .expect("insert test logs");

        let logs_count = log_row_count(logs_db_path(codex_home.as_path()).as_path()).await;

        assert_eq!(logs_count, 1);

        let _ = tokio::fs::remove_dir_all(codex_home).await;
    }

    #[tokio::test]
    async fn init_configures_logs_db_with_incremental_auto_vacuum() {
        let codex_home = unique_temp_dir();
        let _runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
            .await
            .expect("initialize runtime");

        let pool = open_db_pool(logs_db_path(codex_home.as_path()).as_path()).await;
        let auto_vacuum = sqlx::query_scalar::<_, i64>("PRAGMA auto_vacuum")
            .fetch_one(&pool)
            .await
            .expect("read auto_vacuum pragma");
        assert_eq!(auto_vacuum, 2);
        pool.close().await;

        let _ = tokio::fs::remove_dir_all(codex_home).await;
    }

    #[tokio::test]
    async fn query_logs_filters_level_set_without_rewriting_stored_level() {
        let codex_home = unique_temp_dir();
        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
            .await
            .expect("initialize runtime");

        runtime
            .insert_logs(&[
                LogEntry {
                    ts: 1,
                    ts_nanos: 0,
                    level: "TRACE".to_string(),
                    target: "cli".to_string(),
                    message: Some("trace-row".to_string()),
                    thread_id: None,
                    process_uuid: None,
                    file: Some("main.rs".to_string()),
                    line: Some(1),
                    module_path: None,
                },
                LogEntry {
                    ts: 2,
                    ts_nanos: 0,
                    level: "INFO".to_string(),
                    target: "cli".to_string(),
                    message: Some("info-row".to_string()),
                    thread_id: None,
                    process_uuid: None,
                    file: Some("main.rs".to_string()),
                    line: Some(2),
                    module_path: None,
                },
                LogEntry {
                    ts: 3,
                    ts_nanos: 0,
                    level: "warn".to_string(),
                    target: "cli".to_string(),
                    message: Some("warn-row".to_string()),
                    thread_id: None,
                    process_uuid: None,
                    file: Some("main.rs".to_string()),
                    line: Some(3),
                    module_path: None,
                },
                LogEntry {
                    ts: 4,
                    ts_nanos: 0,
                    level: "ERROR".to_string(),
                    target: "cli".to_string(),
                    message: Some("error-row".to_string()),
                    thread_id: None,
                    process_uuid: None,
                    file: Some("main.rs".to_string()),
                    line: Some(4),
                    module_path: None,
                },
            ])
            .await
            .expect("insert test logs");

        let rows = runtime
            .query_logs(&LogQuery {
                levels_upper: vec!["WARN".to_string(), "ERROR".to_string()],
                ..Default::default()
            })
            .await
            .expect("query matching logs");
        let actual = rows
            .iter()
            .map(|row| (row.level.as_str(), row.message.as_deref()))
            .collect::<Vec<_>>();

        assert_eq!(
            actual,
            vec![("warn", Some("warn-row")), ("ERROR", Some("error-row"))]
        );

        let _ = tokio::fs::remove_dir_all(codex_home).await;
    }

    #[tokio::test]
    async fn insert_logs_prunes_old_rows_when_thread_exceeds_size_limit() {
        let codex_home = unique_temp_dir();
        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
            .await
            .expect("initialize runtime");

        let six_mebibytes = "a".repeat(6 * 1024 * 1024);
        runtime
            .insert_logs(&[
                LogEntry {
                    ts: 1,
                    ts_nanos: 0,
                    level: "INFO".to_string(),
                    target: "cli".to_string(),
                    message: Some(six_mebibytes.clone()),
                    thread_id: Some("thread-1".to_string()),
                    process_uuid: Some("proc-1".to_string()),
                    file: Some("main.rs".to_string()),
                    line: Some(1),
                    module_path: Some("mod".to_string()),
                },
                LogEntry {
                    ts: 2,
                    ts_nanos: 0,
                    level: "INFO".to_string(),
                    target: "cli".to_string(),
                    message: Some(six_mebibytes),
                    thread_id: Some("thread-1".to_string()),
                    process_uuid: Some("proc-1".to_string()),
                    file: Some("main.rs".to_string()),
                    line: Some(2),
                    module_path: Some("mod".to_string()),
                },
            ])
            .await
            .expect("insert test logs");

        let rows = runtime
            .query_logs(&LogQuery {
                thread_ids: vec!["thread-1".to_string()],
                ..Default::default()
            })
            .await
            .expect("query thread logs");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].ts, 2);

        let _ = tokio::fs::remove_dir_all(codex_home).await;
    }

    #[tokio::test]
    async fn insert_logs_prunes_single_thread_row_when_it_exceeds_size_limit() {
        let codex_home = unique_temp_dir();
        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
            .await
            .expect("initialize runtime");

        let eleven_mebibytes = "d".repeat(11 * 1024 * 1024);
        runtime
            .insert_logs(&[LogEntry {
                ts: 1,
                ts_nanos: 0,
                level: "INFO".to_string(),
                target: "cli".to_string(),
                message: Some(eleven_mebibytes),
                thread_id: Some("thread-oversized".to_string()),
                process_uuid: Some("proc-1".to_string()),
                file: Some("main.rs".to_string()),
                line: Some(1),
                module_path: Some("mod".to_string()),
            }])
            .await
            .expect("insert test log");

        let rows = runtime
            .query_logs(&LogQuery {
                thread_ids: vec!["thread-oversized".to_string()],
                ..Default::default()
            })
            .await
            .expect("query thread logs");

        assert!(rows.is_empty());

        let _ = tokio::fs::remove_dir_all(codex_home).await;
    }

    #[tokio::test]
    async fn insert_logs_prunes_threadless_rows_per_process_uuid_only() {
        let codex_home = unique_temp_dir();
        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
            .await
            .expect("initialize runtime");

        let six_mebibytes = "b".repeat(6 * 1024 * 1024);
        runtime
            .insert_logs(&[
                LogEntry {
                    ts: 1,
                    ts_nanos: 0,
                    level: "INFO".to_string(),
                    target: "cli".to_string(),
                    message: Some(six_mebibytes.clone()),
                    thread_id: None,
                    process_uuid: Some("proc-1".to_string()),
                    file: Some("main.rs".to_string()),
                    line: Some(1),
                    module_path: Some("mod".to_string()),
                },
                LogEntry {
                    ts: 2,
                    ts_nanos: 0,
                    level: "INFO".to_string(),
                    target: "cli".to_string(),
                    message: Some(six_mebibytes.clone()),
                    thread_id: None,
                    process_uuid: Some("proc-1".to_string()),
                    file: Some("main.rs".to_string()),
                    line: Some(2),
                    module_path: Some("mod".to_string()),
                },
                LogEntry {
                    ts: 3,
                    ts_nanos: 0,
                    level: "INFO".to_string(),
                    target: "cli".to_string(),
                    message: Some(six_mebibytes),
                    thread_id: Some("thread-1".to_string()),
                    process_uuid: Some("proc-1".to_string()),
                    file: Some("main.rs".to_string()),
                    line: Some(3),
                    module_path: Some("mod".to_string()),
                },
            ])
            .await
            .expect("insert test logs");

        let rows = runtime
            .query_logs(&LogQuery {
                thread_ids: vec!["thread-1".to_string()],
                include_threadless: true,
                ..Default::default()
            })
            .await
            .expect("query thread and threadless logs");

        let mut timestamps: Vec<i64> = rows.into_iter().map(|row| row.ts).collect();
        timestamps.sort_unstable();
        assert_eq!(timestamps, vec![2, 3]);

        let _ = tokio::fs::remove_dir_all(codex_home).await;
    }

    #[tokio::test]
    async fn insert_logs_prunes_single_threadless_process_row_when_it_exceeds_size_limit() {
        let codex_home = unique_temp_dir();
        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
            .await
            .expect("initialize runtime");

        let eleven_mebibytes = "e".repeat(11 * 1024 * 1024);
        runtime
            .insert_logs(&[LogEntry {
                ts: 1,
                ts_nanos: 0,
                level: "INFO".to_string(),
                target: "cli".to_string(),
                message: Some(eleven_mebibytes),
                thread_id: None,
                process_uuid: Some("proc-oversized".to_string()),
                file: Some("main.rs".to_string()),
                line: Some(1),
                module_path: Some("mod".to_string()),
            }])
            .await
            .expect("insert test log");

        let rows = runtime
            .query_logs(&LogQuery {
                include_threadless: true,
                ..Default::default()
            })
            .await
            .expect("query threadless logs");

        assert!(rows.is_empty());

        let _ = tokio::fs::remove_dir_all(codex_home).await;
    }

    #[tokio::test]
    async fn insert_logs_prunes_threadless_rows_with_null_process_uuid() {
        let codex_home = unique_temp_dir();
        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
            .await
            .expect("initialize runtime");

        let six_mebibytes = "c".repeat(6 * 1024 * 1024);
        runtime
            .insert_logs(&[
                LogEntry {
                    ts: 1,
                    ts_nanos: 0,
                    level: "INFO".to_string(),
                    target: "cli".to_string(),
                    message: Some(six_mebibytes.clone()),
                    thread_id: None,
                    process_uuid: None,
                    file: Some("main.rs".to_string()),
                    line: Some(1),
                    module_path: Some("mod".to_string()),
                },
                LogEntry {
                    ts: 2,
                    ts_nanos: 0,
                    level: "INFO".to_string(),
                    target: "cli".to_string(),
                    message: Some(six_mebibytes),
                    thread_id: None,
                    process_uuid: None,
                    file: Some("main.rs".to_string()),
                    line: Some(2),
                    module_path: Some("mod".to_string()),
                },
                LogEntry {
                    ts: 3,
                    ts_nanos: 0,
                    level: "INFO".to_string(),
                    target: "cli".to_string(),
                    message: Some("small".to_string()),
                    thread_id: None,
                    process_uuid: Some("proc-1".to_string()),
                    file: Some("main.rs".to_string()),
                    line: Some(3),
                    module_path: Some("mod".to_string()),
                },
            ])
            .await
            .expect("insert test logs");

        let rows = runtime
            .query_logs(&LogQuery {
                include_threadless: true,
                ..Default::default()
            })
            .await
            .expect("query threadless logs");

        let mut timestamps: Vec<i64> = rows.into_iter().map(|row| row.ts).collect();
        timestamps.sort_unstable();
        assert_eq!(timestamps, vec![2, 3]);

        let _ = tokio::fs::remove_dir_all(codex_home).await;
    }

    #[tokio::test]
    async fn insert_logs_prunes_single_threadless_null_process_row_when_it_exceeds_limit() {
        let codex_home = unique_temp_dir();
        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
            .await
            .expect("initialize runtime");

        let eleven_mebibytes = "f".repeat(11 * 1024 * 1024);
        runtime
            .insert_logs(&[LogEntry {
                ts: 1,
                ts_nanos: 0,
                level: "INFO".to_string(),
                target: "cli".to_string(),
                message: Some(eleven_mebibytes),
                thread_id: None,
                process_uuid: None,
                file: Some("main.rs".to_string()),
                line: Some(1),
                module_path: Some("mod".to_string()),
            }])
            .await
            .expect("insert test log");

        let rows = runtime
            .query_logs(&LogQuery {
                include_threadless: true,
                ..Default::default()
            })
            .await
            .expect("query threadless logs");

        assert!(rows.is_empty());

        let _ = tokio::fs::remove_dir_all(codex_home).await;
    }

    #[tokio::test]
    async fn insert_logs_prunes_old_rows_when_thread_exceeds_row_limit() {
        let codex_home = unique_temp_dir();
        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
            .await
            .expect("initialize runtime");

        let entries: Vec<LogEntry> = (1..=1_001)
            .map(|ts| LogEntry {
                ts,
                ts_nanos: 0,
                level: "INFO".to_string(),
                target: "cli".to_string(),
                message: Some(format!("thread-row-{ts}")),
                thread_id: Some("thread-row-limit".to_string()),
                process_uuid: Some("proc-1".to_string()),
                file: Some("main.rs".to_string()),
                line: Some(ts),
                module_path: Some("mod".to_string()),
            })
            .collect();
        runtime
            .insert_logs(&entries)
            .await
            .expect("insert test logs");

        let rows = runtime
            .query_logs(&LogQuery {
                thread_ids: vec!["thread-row-limit".to_string()],
                ..Default::default()
            })
            .await
            .expect("query thread logs");

        let timestamps: Vec<i64> = rows.into_iter().map(|row| row.ts).collect();
        assert_eq!(timestamps.len(), 1_000);
        assert_eq!(timestamps.first().copied(), Some(2));
        assert_eq!(timestamps.last().copied(), Some(1_001));

        let _ = tokio::fs::remove_dir_all(codex_home).await;
    }

    #[tokio::test]
    async fn insert_logs_prunes_old_threadless_rows_when_process_exceeds_row_limit() {
        let codex_home = unique_temp_dir();
        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
            .await
            .expect("initialize runtime");

        let entries: Vec<LogEntry> = (1..=1_001)
            .map(|ts| LogEntry {
                ts,
                ts_nanos: 0,
                level: "INFO".to_string(),
                target: "cli".to_string(),
                message: Some(format!("process-row-{ts}")),
                thread_id: None,
                process_uuid: Some("proc-row-limit".to_string()),
                file: Some("main.rs".to_string()),
                line: Some(ts),
                module_path: Some("mod".to_string()),
            })
            .collect();
        runtime
            .insert_logs(&entries)
            .await
            .expect("insert test logs");

        let rows = runtime
            .query_logs(&LogQuery {
                include_threadless: true,
                ..Default::default()
            })
            .await
            .expect("query threadless logs");

        let timestamps: Vec<i64> = rows
            .into_iter()
            .filter(|row| row.process_uuid.as_deref() == Some("proc-row-limit"))
            .map(|row| row.ts)
            .collect();
        assert_eq!(timestamps.len(), 1_000);
        assert_eq!(timestamps.first().copied(), Some(2));
        assert_eq!(timestamps.last().copied(), Some(1_001));

        let _ = tokio::fs::remove_dir_all(codex_home).await;
    }

    #[tokio::test]
    async fn insert_logs_prunes_old_threadless_null_process_rows_when_row_limit_exceeded() {
        let codex_home = unique_temp_dir();
        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
            .await
            .expect("initialize runtime");

        let entries: Vec<LogEntry> = (1..=1_001)
            .map(|ts| LogEntry {
                ts,
                ts_nanos: 0,
                level: "INFO".to_string(),
                target: "cli".to_string(),
                message: Some(format!("null-process-row-{ts}")),
                thread_id: None,
                process_uuid: None,
                file: Some("main.rs".to_string()),
                line: Some(ts),
                module_path: Some("mod".to_string()),
            })
            .collect();
        runtime
            .insert_logs(&entries)
            .await
            .expect("insert test logs");

        let rows = runtime
            .query_logs(&LogQuery {
                include_threadless: true,
                ..Default::default()
            })
            .await
            .expect("query threadless logs");

        let timestamps: Vec<i64> = rows
            .into_iter()
            .filter(|row| row.process_uuid.is_none())
            .map(|row| row.ts)
            .collect();
        assert_eq!(timestamps.len(), 1_000);
        assert_eq!(timestamps.first().copied(), Some(2));
        assert_eq!(timestamps.last().copied(), Some(1_001));

        let _ = tokio::fs::remove_dir_all(codex_home).await;
    }
}
