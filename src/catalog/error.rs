use thiserror::Error;

use crate::{
    catalog::table::TableId,
    storage::{index::error::IndexError, table::error::TableHeapError},
};

#[derive(Debug)]
pub enum TableIdentifier {
    Name(String),
    Oid(TableId),
}

impl From<&str> for TableIdentifier {
    fn from(s: &str) -> Self {
        TableIdentifier::Name(s.into())
    }
}

impl From<TableId> for TableIdentifier {
    fn from(oid: TableId) -> Self {
        TableIdentifier::Oid(oid)
    }
}

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("table with name {0} already exists")]
    DuplicateTable(String),

    #[error("index with name {0} already exists")]
    DuplicateIndex(String),

    #[error("buffer pool error: {0}")]
    TableHeap(#[from] TableHeapError),

    #[error("table not found: {0:?}")]
    TableNotFound(TableIdentifier),

    #[error("unsupported index type")]
    UnsupportedIndexType,

    #[error("index error: {0}")]
    Index(#[from] IndexError),
}
