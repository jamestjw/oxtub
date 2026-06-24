use thiserror::Error;

use crate::catalog::error::CatalogError;

#[derive(Debug, Error)]
pub enum BinderError {
    #[error("duplicate column: {0}")]
    DuplicateColumn(String),

    #[error("primary key column not found: {0}")]
    PrimaryKeyColumnNotFound(String),

    #[error("duplicate primary key column: {0}")]
    DuplicatePrimaryKeyColumn(String),

    #[error("creating table without columns")]
    CreateTableWithoutColumns,

    #[error("table not found: {0}")]
    TableNotFound(String),

    #[error("illegal table name: {0}")]
    InvalidTableName(String),

    #[error("catalog error: {0}")]
    Catalog(#[from] CatalogError),

    #[error("values cannot be empty")]
    InsertValuesEmpty,

    #[error("values must match columns")]
    InsertValuesDoesntMatchColumns,

    #[error("unsupported expression: {0}")]
    UnsupportedExpression(String),

    #[error("col is ambiguous in schema: {0}")]
    AmbiguousColumn(String),

    #[error("duplicated columns in insert")]
    DuplicateInsertColumns,

    #[error("column not found: {0}")]
    ColumnNotFound(String),

    #[error("must select something")]
    EmptySelectProjection,
}
