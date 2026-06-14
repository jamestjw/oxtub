use thiserror::Error;

#[derive(Debug, Error)]
pub enum ValueError {
    #[error("invalid operation for value type")]
    InvalidOperation,
}
