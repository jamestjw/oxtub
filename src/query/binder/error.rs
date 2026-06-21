use thiserror::Error;

#[derive(Debug, Error)]
pub enum BinderError {
    #[error("duplicate column: {0}")]
    DuplicateColumn(String),

    #[error("primary key column not found: {0}")]
    PrimaryKeyColumnNotFound(String),

    #[error("duplicate primary key column: {0}")]
    DuplicatePrimaryKeyColumn(String),
}
