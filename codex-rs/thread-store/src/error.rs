use codex_protocol::ThreadId;

pub type ThreadStoreResult<T> = Result<T, ThreadStoreError>;

#[derive(Debug, thiserror::Error)]
pub enum ThreadStoreError {
    #[error("thread {thread_id} not found")]
    ThreadNotFound { thread_id: ThreadId },

    #[error("invalid thread-store request: {message}")]
    InvalidRequest { message: String },

    #[error("thread-store conflict: {message}")]
    Conflict { message: String },

    #[error("thread-store unsupported operation: {operation}")]
    Unsupported { operation: &'static str },

    #[error("thread-store internal error: {message}")]
    Internal { message: String },
}
