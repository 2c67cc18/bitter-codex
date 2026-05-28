use std::sync::LazyLock;

use codex_protocol::protocol::SessionSource;

pub(crate) mod config;
pub(crate) mod list;
pub(crate) mod metadata;
pub(crate) mod policy;
pub(crate) mod recorder;
pub(crate) mod search;
mod sqlite_metrics;
pub mod state_db;

pub(crate) mod default_client {
    pub use codex_login::default_client::*;
}

pub(crate) use codex_protocol::protocol;

pub const SESSIONS_SUBDIR: &str = "sessions";
pub const ARCHIVED_SESSIONS_SUBDIR: &str = "archived_sessions";
pub static INTERACTIVE_SESSION_SOURCES: LazyLock<Vec<SessionSource>> = LazyLock::new(|| {
    vec![
        SessionSource::Cli,
        SessionSource::VSCode,
        SessionSource::Custom("atlas".to_string()),
        SessionSource::Custom("chatgpt".to_string()),
    ]
});

pub use codex_protocol::protocol::SessionMeta;
pub use config::Config;
pub use config::RolloutConfig;
pub use config::RolloutConfigView;
pub use list::Cursor;
pub use list::SortDirection;
pub use list::ThreadItem;
pub use list::ThreadListConfig;
pub use list::ThreadListLayout;
pub use list::ThreadSortKey;
pub use list::ThreadsPage;
pub use list::find_archived_thread_path_by_id_str;
pub use list::find_thread_path_by_id_str;
pub use list::get_threads;
pub use list::get_threads_in_root;
pub use list::parse_cursor;
pub use list::read_head_for_summary;
pub use list::read_session_meta_line;
pub use list::read_thread_item_from_rollout;
pub use list::rollout_date_parts;
pub use metadata::builder_from_items;
pub use policy::EventPersistenceMode;
pub use policy::is_persisted_rollout_item;
pub use policy::persisted_rollout_items;
pub use recorder::RolloutRecorder;
pub use recorder::RolloutRecorderParams;
pub use recorder::append_rollout_item_to_path;
pub use search::first_rollout_content_match_snippet;
pub use search::search_rollout_paths;
pub use state_db::StateDbHandle;
pub use state_db::sqlite_telemetry_recorder;
