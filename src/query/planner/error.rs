use thiserror::Error;

use crate::catalog::error::CatalogError;

#[derive(Debug, Error)]
pub enum PlannerError {
    #[error("unsupported statement")]
    UnsupportedStatement,

    #[error("catalog error: {0}")]
    Catalog(#[from] CatalogError),

    #[error("more than one column with name: {0}")]
    AmbiguousColumn(String),

    #[error("insert schema mismatch")]
    InsertSchemaMismatch,

    #[error("update schema mismatch")]
    UpdateSchemaMismatch,

    #[error("nested aggregate expressions are not supported")]
    NestedAggregate,

    #[error("aggregate reference is not valid in this expression scope")]
    UnexpectedAggregateReference,
}
