use thiserror::Error;

use crate::storage::table::error::TableHeapError;

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("table with name {0} already exists")]
    DuplicateTableName(String),

    #[error("buffer pool error: {0}")]
    TableHeap(#[from] TableHeapError),
}
