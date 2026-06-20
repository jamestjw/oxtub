use thiserror::Error;

use crate::{
    catalog::{index::IndexId, table::TableId},
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

#[derive(Debug)]
pub enum IndexIdentifier {
    Name(String),
    Oid(IndexId),
}

impl From<&str> for IndexIdentifier {
    fn from(s: &str) -> Self {
        Self::Name(s.into())
    }
}

impl From<IndexId> for IndexIdentifier {
    fn from(oid: IndexId) -> Self {
        Self::Oid(oid)
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

    #[error("index not found: {0:?}")]
    IndexNotFound(IndexIdentifier),
}
