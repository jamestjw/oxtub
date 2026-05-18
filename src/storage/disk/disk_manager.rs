use crate::storage::disk::error::DiskManagerError;

use super::config::DEFAULT_DB_FILE_PAGE_CAPACITY;
use std::fs::{File, OpenOptions};

pub struct DiskManager {
    db_file: std::path::PathBuf,
    db_file_handle: File,
    page_capacity: usize,
}

impl DiskManager {
    pub fn new(db_file: std::path::PathBuf) -> Result<DiskManager, DiskManagerError> {
        let db_file_handle = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&db_file)?;

        Ok(Self {
            db_file,
            db_file_handle,
            page_capacity: DEFAULT_DB_FILE_PAGE_CAPACITY,
        })
    }
}
