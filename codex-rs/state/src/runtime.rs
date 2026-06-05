use crate::LOGS_DB_FILENAME;
use crate::LogEntry;
use crate::LogQuery;
use crate::LogRow;
use crate::STATE_DB_FILENAME;
use crate::SortKey;
use crate::ThreadMetadata;
use crate::ThreadMetadataBuilder;
use crate::ThreadsPage;
use crate::apply_rollout_item;
use crate::model::ThreadRow;
use crate::model::anchor_from_item;
use crate::model::datetime_to_epoch_millis;
use crate::model::datetime_to_epoch_seconds;
use crate::model::epoch_millis_to_datetime;
use crate::paths::file_modified_time_utc;
use crate::telemetry::DbKind;
use crate::telemetry::DbTelemetry;
use chrono::DateTime;
use chrono::Utc;
use codex_protocol::ThreadId;
use codex_protocol::protocol::RolloutItem;
use log::LevelFilter;
use sqlx::ConnectOptions;
use sqlx::QueryBuilder;
use sqlx::Row;
use sqlx::Sqlite;
use sqlx::SqliteConnection;
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteAutoVacuum;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::sqlite::SqliteJournalMode;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::sqlite::SqliteSynchronous;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicI64;
use std::time::Duration;
use std::time::Instant;
use tracing::warn;

mod backfill;
mod logs;
#[cfg(test)]
mod test_support;
mod threads;

pub use threads::ThreadFilterOptions;

const LOG_PARTITION_SIZE_LIMIT_BYTES: i64 = 10 * 1024 * 1024;
const LOG_PARTITION_ROW_LIMIT: i64 = 1_000;

#[derive(Clone, Copy)]
struct RuntimeDbSpec {
    label: &'static str,
    filename: &'static str,
    kind: DbKind,
    open_phase: &'static str,
}

impl RuntimeDbSpec {
    fn path(self, codex_home: &Path) -> PathBuf {
        codex_home.join(self.filename)
    }
}

const STATE_DB: RuntimeDbSpec = RuntimeDbSpec {
    label: "state DB",
    filename: STATE_DB_FILENAME,
    kind: DbKind::State,
    open_phase: "open_state",
};

const LOGS_DB: RuntimeDbSpec = RuntimeDbSpec {
    label: "log DB",
    filename: LOGS_DB_FILENAME,
    kind: DbKind::Logs,
    open_phase: "open_logs",
};

const RUNTIME_DBS: [RuntimeDbSpec; 2] = [STATE_DB, LOGS_DB];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeDbPath {
    pub label: &'static str,
    pub path: PathBuf,
}

#[derive(Clone)]
pub struct StateRuntime {
    codex_home: PathBuf,
    default_provider: String,
    pool: Arc<sqlx::SqlitePool>,
    logs_pool: Arc<sqlx::SqlitePool>,
    thread_updated_at_millis: Arc<AtomicI64>,
}

impl StateRuntime {
    pub async fn init(codex_home: PathBuf, default_provider: String) -> anyhow::Result<Arc<Self>> {
        Self::init_inner(codex_home, default_provider, None).await
    }

    #[cfg(test)]
    pub(crate) async fn init_with_telemetry_for_tests(
        codex_home: PathBuf,
        default_provider: String,
        telemetry_override: &dyn DbTelemetry,
    ) -> anyhow::Result<Arc<Self>> {
        Self::init_inner(codex_home, default_provider, Some(telemetry_override)).await
    }

    async fn init_inner(
        codex_home: PathBuf,
        default_provider: String,
        telemetry_override: Option<&dyn DbTelemetry>,
    ) -> anyhow::Result<Arc<Self>> {
        tokio::fs::create_dir_all(&codex_home).await?;
        let state_path = STATE_DB.path(codex_home.as_path());
        let logs_path = LOGS_DB.path(codex_home.as_path());
        let pool = match open_state_sqlite(&state_path, telemetry_override).await {
            Ok(db) => Arc::new(db),
            Err(err) => {
                warn!("failed to open state db at {}: {err}", state_path.display());
                return Err(err);
            }
        };
        let started = Instant::now();
        let state_schema_result = ensure_state_schema(pool.as_ref()).await;
        crate::telemetry::record_init_result(
            telemetry_override,
            DbKind::State,
            "create_state_schema",
            started.elapsed(),
            &state_schema_result,
        );
        state_schema_result?;
        let logs_pool = match open_logs_sqlite(&logs_path, telemetry_override).await {
            Ok(db) => Arc::new(db),
            Err(err) => {
                warn!("failed to open logs db at {}: {err}", logs_path.display());
                return Err(err);
            }
        };
        let started = Instant::now();
        let logs_schema_result = ensure_logs_schema(logs_pool.as_ref()).await;
        crate::telemetry::record_init_result(
            telemetry_override,
            DbKind::Logs,
            "create_logs_schema",
            started.elapsed(),
            &logs_schema_result,
        );
        logs_schema_result?;
        let started = Instant::now();
        let backfill_state_result = ensure_backfill_state_row_in_pool(pool.as_ref()).await;
        crate::telemetry::record_init_result(
            telemetry_override,
            DbKind::State,
            "ensure_backfill_state",
            started.elapsed(),
            &backfill_state_result,
        );
        backfill_state_result?;
        let started = Instant::now();
        let thread_updated_at_millis_result: anyhow::Result<Option<i64>> =
            sqlx::query_scalar("SELECT MAX(threads.updated_at_ms) FROM threads")
                .fetch_one(pool.as_ref())
                .await
                .map_err(anyhow::Error::from);
        crate::telemetry::record_init_result(
            telemetry_override,
            DbKind::State,
            "post_init_query",
            started.elapsed(),
            &thread_updated_at_millis_result,
        );
        let thread_updated_at_millis = thread_updated_at_millis_result?;
        let thread_updated_at_millis = thread_updated_at_millis.unwrap_or(0);
        let runtime = Arc::new(Self {
            pool,
            logs_pool,
            codex_home,
            default_provider,
            thread_updated_at_millis: Arc::new(AtomicI64::new(thread_updated_at_millis)),
        });
        if let Err(err) = runtime.run_logs_startup_maintenance().await {
            warn!(
                "failed to run startup maintenance for logs db at {}: {err}",
                logs_path.display(),
            );
        }
        Ok(runtime)
    }

    pub fn codex_home(&self) -> &Path {
        self.codex_home.as_path()
    }
}

fn base_sqlite_options(path: &Path) -> SqliteConnectOptions {
    SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5))
        .log_statements(LevelFilter::Off)
}

async fn open_state_sqlite(
    path: &Path,
    telemetry_override: Option<&dyn DbTelemetry>,
) -> anyhow::Result<SqlitePool> {
    open_sqlite(path, STATE_DB, telemetry_override).await
}

async fn open_logs_sqlite(
    path: &Path,
    telemetry_override: Option<&dyn DbTelemetry>,
) -> anyhow::Result<SqlitePool> {
    open_sqlite(path, LOGS_DB, telemetry_override).await
}

async fn open_sqlite(
    path: &Path,
    spec: RuntimeDbSpec,
    telemetry_override: Option<&dyn DbTelemetry>,
) -> anyhow::Result<SqlitePool> {
    let options = base_sqlite_options(path).auto_vacuum(SqliteAutoVacuum::Incremental);
    let started = Instant::now();
    let pool_result = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .map_err(anyhow::Error::from);
    crate::telemetry::record_init_result(
        telemetry_override,
        spec.kind,
        spec.open_phase,
        started.elapsed(),
        &pool_result,
    );
    let pool = pool_result?;
    Ok(pool)
}

async fn ensure_state_schema(pool: &sqlx::SqlitePool) -> anyhow::Result<()> {
    sqlx::query(
        r#"
CREATE TABLE IF NOT EXISTS threads (
    id TEXT PRIMARY KEY,
    rollout_path TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    source TEXT NOT NULL,
    model_provider TEXT NOT NULL,
    model TEXT,
    reasoning_effort TEXT,
    cwd TEXT NOT NULL,
    cli_version TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL,
    preview TEXT NOT NULL DEFAULT '',
    tokens_used INTEGER NOT NULL DEFAULT 0,
    first_user_message TEXT NOT NULL DEFAULT '',
    archived INTEGER NOT NULL DEFAULT 0,
    archived_at INTEGER,
    git_sha TEXT,
    git_branch TEXT,
    git_origin_url TEXT
)
        "#,
    )
    .execute(pool)
    .await?;
    sqlx::query(
        r#"
CREATE TABLE IF NOT EXISTS thread_dynamic_tools (
    thread_id TEXT NOT NULL,
    position INTEGER NOT NULL,
    namespace TEXT NOT NULL DEFAULT '',
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    input_schema TEXT NOT NULL,
    defer_loading INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(thread_id, position),
    FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
)
        "#,
    )
    .execute(pool)
    .await?;
    sqlx::query(
        r#"
CREATE TABLE IF NOT EXISTS backfill_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    status TEXT NOT NULL,
    last_watermark TEXT,
    last_success_at INTEGER,
    updated_at INTEGER NOT NULL
)
        "#,
    )
    .execute(pool)
    .await?;
    for statement in [
        "CREATE INDEX IF NOT EXISTS idx_threads_created_at_ms ON threads(created_at_ms DESC, id DESC)",
        "CREATE INDEX IF NOT EXISTS idx_threads_updated_at_ms ON threads(updated_at_ms DESC, id DESC)",
        "CREATE INDEX IF NOT EXISTS idx_threads_archived ON threads(archived)",
        "CREATE INDEX IF NOT EXISTS idx_threads_source ON threads(source)",
        "CREATE INDEX IF NOT EXISTS idx_threads_provider ON threads(model_provider)",
        "CREATE INDEX IF NOT EXISTS idx_threads_archived_cwd_created_at_ms ON threads(archived, cwd, created_at_ms DESC, id DESC)",
        "CREATE INDEX IF NOT EXISTS idx_threads_archived_cwd_updated_at_ms ON threads(archived, cwd, updated_at_ms DESC, id DESC)",
        "CREATE INDEX IF NOT EXISTS idx_thread_dynamic_tools_thread ON thread_dynamic_tools(thread_id)",
    ] {
        sqlx::query(statement).execute(pool).await?;
    }
    Ok(())
}

async fn ensure_logs_schema(pool: &sqlx::SqlitePool) -> anyhow::Result<()> {
    sqlx::query(
        r#"
CREATE TABLE IF NOT EXISTS logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts INTEGER NOT NULL,
    ts_nanos INTEGER NOT NULL,
    level TEXT NOT NULL,
    target TEXT NOT NULL,
    message TEXT,
    module_path TEXT,
    file TEXT,
    line INTEGER,
    thread_id TEXT,
    process_uuid TEXT,
    estimated_bytes INTEGER NOT NULL DEFAULT 0
)
        "#,
    )
    .execute(pool)
    .await?;
    for statement in [
        "CREATE INDEX IF NOT EXISTS idx_logs_ts ON logs(ts DESC, ts_nanos DESC, id DESC)",
        "CREATE INDEX IF NOT EXISTS idx_logs_thread_id ON logs(thread_id)",
        "CREATE INDEX IF NOT EXISTS idx_logs_process_uuid ON logs(process_uuid)",
        "CREATE INDEX IF NOT EXISTS idx_logs_thread_id_ts ON logs(thread_id, ts DESC, ts_nanos DESC, id DESC)",
        "CREATE INDEX IF NOT EXISTS idx_logs_process_uuid_threadless_ts ON logs(process_uuid, ts DESC, ts_nanos DESC, id DESC) WHERE thread_id IS NULL",
    ] {
        sqlx::query(statement).execute(pool).await?;
    }
    Ok(())
}

pub(super) async fn ensure_backfill_state_row_in_pool(
    pool: &sqlx::SqlitePool,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
INSERT INTO backfill_state (id, status, last_watermark, last_success_at, updated_at)
VALUES (?, ?, NULL, NULL, ?)
ON CONFLICT(id) DO NOTHING
            "#,
    )
    .bind(1_i64)
    .bind(crate::BackfillStatus::Pending.as_str())
    .bind(Utc::now().timestamp())
    .execute(pool)
    .await?;
    Ok(())
}

pub fn state_db_filename() -> String {
    STATE_DB.filename.to_string()
}

pub fn state_db_path(codex_home: &Path) -> PathBuf {
    STATE_DB.path(codex_home)
}

pub fn logs_db_filename() -> String {
    LOGS_DB.filename.to_string()
}

pub fn logs_db_path(codex_home: &Path) -> PathBuf {
    LOGS_DB.path(codex_home)
}

pub fn runtime_db_paths(codex_home: &Path) -> Vec<RuntimeDbPath> {
    RUNTIME_DBS
        .iter()
        .map(|spec| RuntimeDbPath {
            label: spec.label,
            path: spec.path(codex_home),
        })
        .collect()
}

pub async fn sqlite_integrity_check(path: &Path) -> anyhow::Result<Vec<String>> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .read_only(true)
        .log_statements(LevelFilter::Off);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;
    let rows = sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
        .fetch_all(&pool)
        .await?;
    pool.close().await;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::StateRuntime;
    use super::sqlite_integrity_check;
    use super::state_db_path;
    use super::test_support::unique_temp_dir;
    use crate::DB_INIT_METRIC;
    use crate::DbTelemetry;
    use crate::ThreadMetadataBuilder;
    use chrono::Utc;
    use codex_protocol::ThreadId;
    use codex_protocol::dynamic_tools::DynamicToolSpec;
    use codex_protocol::protocol::RolloutItem;
    use codex_protocol::protocol::SessionMeta;
    use codex_protocol::protocol::SessionMetaLine;
    use codex_protocol::protocol::SessionSource;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use sqlx::SqlitePool;
    use sqlx::sqlite::SqliteConnectOptions;
    use std::collections::BTreeMap;
    use std::collections::BTreeSet;
    use std::sync::Mutex;

    #[derive(Default)]
    struct TestTelemetry {
        counters: Mutex<Vec<MetricEvent>>,
    }

    #[derive(Debug, Eq, PartialEq)]
    struct MetricEvent {
        name: String,
        tags: BTreeMap<String, String>,
    }

    impl TestTelemetry {
        fn counters(&self) -> Vec<MetricEvent> {
            self.counters
                .lock()
                .expect("telemetry lock")
                .iter()
                .map(|event| MetricEvent {
                    name: event.name.clone(),
                    tags: event.tags.clone(),
                })
                .collect()
        }
    }

    impl DbTelemetry for TestTelemetry {
        fn counter(&self, name: &str, _inc: i64, tags: &[(&str, &str)]) {
            self.counters
                .lock()
                .expect("telemetry lock")
                .push(MetricEvent {
                    name: name.to_string(),
                    tags: tags_to_map(tags),
                });
        }

        fn record_duration(
            &self,
            _name: &str,
            _duration: std::time::Duration,
            _tags: &[(&str, &str)],
        ) {
        }
    }

    fn tags_to_map(tags: &[(&str, &str)]) -> BTreeMap<String, String> {
        tags.iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    #[tokio::test]
    async fn sqlite_integrity_check_reports_ok_for_valid_db() {
        let codex_home = unique_temp_dir();
        tokio::fs::create_dir_all(&codex_home)
            .await
            .expect("create codex home");
        let path = state_db_path(codex_home.as_path());
        let pool = SqlitePool::connect_with(
            SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
        )
        .await
        .expect("open sqlite db");
        sqlx::query("CREATE TABLE sample (id INTEGER PRIMARY KEY)")
            .execute(&pool)
            .await
            .expect("create sample table");
        pool.close().await;

        let result = sqlite_integrity_check(&path)
            .await
            .expect("integrity check should run");

        assert_eq!(result, vec!["ok".to_string()]);
        let _ = tokio::fs::remove_dir_all(codex_home).await;
    }

    #[tokio::test]
    async fn apply_rollout_items_does_not_write_thread_dynamic_tools() {
        let codex_home = unique_temp_dir();
        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
            .await
            .expect("initialize runtime");
        let thread_id = ThreadId::new();
        let rollout_path = codex_home.join("rollout.jsonl");
        let builder =
            ThreadMetadataBuilder::new(thread_id, rollout_path, Utc::now(), SessionSource::Exec);
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

        runtime
            .apply_rollout_items(
                &builder,
                &[RolloutItem::SessionMeta(SessionMetaLine {
                    meta: SessionMeta {
                        id: thread_id,
                        dynamic_tools: Some(dynamic_tools),
                        ..Default::default()
                    },
                    git: None,
                })],
                None,
            )
            .await
            .expect("apply rollout items");

        let stored_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM thread_dynamic_tools WHERE thread_id = ?")
                .bind(thread_id.to_string())
                .fetch_one(runtime.pool.as_ref())
                .await
                .expect("count dynamic tools");
        assert_eq!(stored_count, 0);
        let _ = tokio::fs::remove_dir_all(codex_home).await;
    }

    #[tokio::test]
    async fn init_records_successful_sqlite_init_phases_to_explicit_telemetry() {
        let codex_home = unique_temp_dir();
        let telemetry = TestTelemetry::default();

        let runtime = StateRuntime::init_with_telemetry_for_tests(
            codex_home.clone(),
            "test-provider".to_string(),
            &telemetry,
        )
        .await
        .expect("state runtime should initialize");

        let phases = telemetry
            .counters()
            .into_iter()
            .filter(|event| event.name == DB_INIT_METRIC)
            .filter(|event| event.tags.get("status").map(String::as_str) == Some("success"))
            .filter_map(|event| event.tags.get("phase").cloned())
            .collect::<BTreeSet<_>>();
        let expected = [
            "open_state",
            "create_state_schema",
            "open_logs",
            "create_logs_schema",
            "ensure_backfill_state",
            "post_init_query",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
        assert_eq!(phases, expected);

        runtime.pool.close().await;
        runtime.logs_pool.close().await;
        let _ = tokio::fs::remove_dir_all(codex_home).await;
    }
}
