use thiserror::Error;

#[derive(Debug, Error)]
pub enum DiskManagerError {
    #[error("io error {0}")]
    Io(#[from] std::io::Error),
    #[error("does not match expected page size")]
    InvalidPageSize,
    #[error("page {0} not found")]
    PageNotFound(usize),
}
