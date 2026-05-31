use thiserror::Error;

use crate::common::types::PageId;

#[derive(Debug, Error)]
pub enum DiskManagerError {
    #[error("io error {0}")]
    Io(#[from] std::io::Error),
    #[error("does not match expected page size")]
    InvalidPageSize,
    #[error("page {0} not found")]
    PageNotFound(PageId),
}

#[derive(Debug, Error)]
pub enum DiskSchedulerError {
    #[error("io error {0}")]
    Disk(#[from] DiskManagerError),
    #[error("worker stopped")]
    WorkerStopped,
    #[error("worker unreachable")]
    WorkerUnreachable,
}

#[derive(Debug, Error)]
pub enum BufferPoolError {
    #[error("no available frame in buffer pool")]
    NoAvailableFrame,
    #[error("page {0} is pinned")]
    PagePinned(PageId),
    #[error("disk scheduler error: {0}")]
    Disk(#[from] DiskSchedulerError),
}
