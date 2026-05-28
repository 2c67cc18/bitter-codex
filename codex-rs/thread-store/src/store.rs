use async_trait::async_trait;
use codex_protocol::ThreadId;
use std::any::Any;

use crate::AppendThreadItemsParams;
use crate::ArchiveThreadParams;
use crate::CreateThreadParams;
use crate::ItemPage;
use crate::ListItemsParams;
use crate::ListThreadsParams;
use crate::ListTurnsParams;
use crate::LoadThreadHistoryParams;
use crate::ReadThreadByRolloutPathParams;
use crate::ReadThreadParams;
use crate::ResumeThreadParams;
use crate::SearchThreadsParams;
use crate::StoredThread;
use crate::StoredThreadHistory;
use crate::ThreadPage;
use crate::ThreadSearchPage;
use crate::ThreadStoreError;
use crate::ThreadStoreResult;
use crate::TurnPage;
use crate::UpdateThreadMetadataParams;

#[async_trait]
pub trait ThreadStore: Any + Send + Sync {
    fn as_any(&self) -> &dyn Any;

    async fn create_thread(&self, params: CreateThreadParams) -> ThreadStoreResult<()>;

    async fn resume_thread(&self, params: ResumeThreadParams) -> ThreadStoreResult<()>;

    async fn append_items(&self, params: AppendThreadItemsParams) -> ThreadStoreResult<()>;

    async fn persist_thread(&self, thread_id: ThreadId) -> ThreadStoreResult<()>;

    async fn flush_thread(&self, thread_id: ThreadId) -> ThreadStoreResult<()>;

    async fn shutdown_thread(&self, thread_id: ThreadId) -> ThreadStoreResult<()>;

    async fn discard_thread(&self, thread_id: ThreadId) -> ThreadStoreResult<()>;

    async fn load_history(
        &self,
        params: LoadThreadHistoryParams,
    ) -> ThreadStoreResult<StoredThreadHistory>;

    async fn read_thread(&self, params: ReadThreadParams) -> ThreadStoreResult<StoredThread>;

    async fn read_thread_by_rollout_path(
        &self,
        params: ReadThreadByRolloutPathParams,
    ) -> ThreadStoreResult<StoredThread>;

    async fn list_threads(&self, params: ListThreadsParams) -> ThreadStoreResult<ThreadPage>;

    async fn search_threads(
        &self,
        _params: SearchThreadsParams,
    ) -> ThreadStoreResult<ThreadSearchPage> {
        Err(ThreadStoreError::Unsupported {
            operation: "thread/search",
        })
    }

    async fn list_turns(&self, _params: ListTurnsParams) -> ThreadStoreResult<TurnPage> {
        Err(ThreadStoreError::Unsupported {
            operation: "list_turns",
        })
    }

    async fn list_items(&self, _params: ListItemsParams) -> ThreadStoreResult<ItemPage> {
        Err(ThreadStoreError::Unsupported {
            operation: "list_items",
        })
    }

    async fn update_thread_metadata(
        &self,
        params: UpdateThreadMetadataParams,
    ) -> ThreadStoreResult<StoredThread>;

    async fn archive_thread(&self, params: ArchiveThreadParams) -> ThreadStoreResult<()>;

    async fn unarchive_thread(
        &self,
        params: ArchiveThreadParams,
    ) -> ThreadStoreResult<StoredThread>;
}
