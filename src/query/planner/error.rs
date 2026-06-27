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
}
