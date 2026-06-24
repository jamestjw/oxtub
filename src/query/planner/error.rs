use thiserror::Error;

use crate::catalog::error::CatalogError;

#[derive(Debug, Error)]
pub enum PlannerError {
    #[error("unsupported statement")]
    UnsupportedStatement,

    #[error("catalog error: {0}")]
    Catalog(#[from] CatalogError),
}
