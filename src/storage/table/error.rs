use thiserror::Error;

use crate::storage::disk::error::BufferPoolError;

#[derive(Debug, Error)]
pub enum TableHeapError {
    #[error("buffer pool error: {0}")]
    BufferPool(#[from] BufferPoolError),

    #[error("tuple does not fit in page")]
    TupleTooLarge,
}
