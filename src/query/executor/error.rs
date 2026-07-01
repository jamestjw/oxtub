use thiserror::Error;

use crate::{
    catalog::error::CatalogError, storage::table::error::TableHeapError, types::value::Value,
};

#[derive(Debug, Error)]
pub enum ExecutionError {
    #[error("catalog error: {0}")]
    Catalog(#[from] CatalogError),

    #[error("table heap error: {0}")]
    TableHeap(#[from] TableHeapError),

    #[error("expected boolean expression, got {0:?}")]
    ExpectedBoolean(Value),

    #[error("expected numeric expression, got {0:?}")]
    ExpectedNumeric(Value),

    #[error("expected integer expression, got {0:?}")]
    ExpectedInteger(Value),

    #[error("missing rid for delete")]
    MissingRid,

    #[error("unsupported expression")]
    UnsupportedExpression,

    #[error("unsupported plan")]
    UnsupportedPlan,
}
