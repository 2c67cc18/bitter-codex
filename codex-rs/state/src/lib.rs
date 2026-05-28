mod extract;
pub mod log_db;
mod model;
mod paths;
mod runtime;
mod telemetry;

pub use model::LogEntry;
pub use model::LogQuery;
pub use model::LogRow;

pub use runtime::StateRuntime;

pub use extract::apply_rollout_item;
pub use extract::rollout_item_affects_thread_metadata;
pub use model::Anchor;
pub use model::BackfillState;
pub use model::BackfillStats;
pub use model::BackfillStatus;
pub use model::ExtractionOutcome;
pub use model::SortDirection;
pub use model::SortKey;
pub use model::ThreadMetadata;
pub use model::ThreadMetadataBuilder;
pub use model::ThreadsPage;
pub use runtime::RuntimeDbPath;
pub use runtime::ThreadFilterOptions;
pub use runtime::logs_db_filename;
pub use runtime::logs_db_path;
pub use runtime::runtime_db_paths;
pub use runtime::sqlite_integrity_check;
pub use runtime::state_db_filename;
pub use runtime::state_db_path;
pub use telemetry::DbTelemetry;
pub use telemetry::DbTelemetryHandle;
pub use telemetry::install_process_db_telemetry;
pub use telemetry::record_backfill_gate;
pub use telemetry::record_fallback;

pub const SQLITE_HOME_ENV: &str = "CODEX_SQLITE_HOME";

pub const LOGS_DB_FILENAME: &str = "logs_2.sqlite";
pub const STATE_DB_FILENAME: &str = "state_5.sqlite";

pub const DB_ERROR_METRIC: &str = "codex.db.error";

pub const DB_METRIC_BACKFILL: &str = "codex.db.backfill";

pub const DB_METRIC_BACKFILL_DURATION_MS: &str = "codex.db.backfill.duration_ms";

pub const DB_INIT_METRIC: &str = "codex.sqlite.init.count";

pub const DB_INIT_DURATION_METRIC: &str = "codex.sqlite.init.duration_ms";

pub const DB_FALLBACK_METRIC: &str = "codex.sqlite.fallback.count";
