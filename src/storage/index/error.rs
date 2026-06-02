use thiserror::Error;

use crate::storage::disk::error::BufferPoolError;

#[derive(Debug, Error)]
pub enum BTreeError {
    #[error("buffer pool error: {0}")]
    BufferPool(#[from] BufferPoolError),

    #[error("operation requires a non-empty tree")]
    EmptyTree,

    #[error("key-value pair already exists")]
    Duplicate,
}
