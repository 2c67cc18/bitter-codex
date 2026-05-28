use async_trait::async_trait;
use codex_utils_absolute_path::AbsolutePathBuf;
use std::io;

pub type FileSystemResult<T> = io::Result<T>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMetadata {
    pub is_directory: bool,
}

#[async_trait]
pub trait ExecutorFileSystem: Send + Sync {
    async fn read_file_text(&self, path: &AbsolutePathBuf) -> FileSystemResult<String>;

    async fn get_metadata(&self, path: &AbsolutePathBuf) -> FileSystemResult<FileMetadata>;
}

