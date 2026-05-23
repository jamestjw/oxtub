use crate::storage::disk::config::DEFAULT_PAGE_SIZE;
use crate::storage::disk::error::DiskManagerError;

use super::config::DEFAULT_DB_FILE_PAGE_CAPACITY;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

pub struct DiskManager {
    db_file: PathBuf,
    db_file_handle: File,
    page_capacity: usize,
    pages: HashMap<usize, usize>, // page_id -> file offset
    free_slots: Vec<usize>,       // free file offsets (from deleted pages)
}

impl DiskManager {
    pub fn new(db_file: PathBuf) -> Result<DiskManager, DiskManagerError> {
        let db_file_handle = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&db_file)?;

        db_file_handle.set_len(((DEFAULT_DB_FILE_PAGE_CAPACITY + 1) * DEFAULT_PAGE_SIZE) as u64)?;

        Ok(Self {
            db_file,
            db_file_handle,
            page_capacity: DEFAULT_DB_FILE_PAGE_CAPACITY,
            pages: HashMap::new(),
            free_slots: Vec::new(),
        })
    }

    // Reading an unallocated page creates it and returns the zero-filled file contents.
    // TODO: see if this behaviour is really necessary, we can always add a new function
    // that just allocates a new page
    pub fn read_page(&mut self, page_id: usize, data: &mut [u8]) -> Result<(), DiskManagerError> {
        if data.len() != DEFAULT_PAGE_SIZE {
            return Err(DiskManagerError::InvalidPageSize);
        }

        // If the page is not allocated yet, it's OK we allocate it on the fly
        let offset = match self.pages.get(&page_id) {
            Some(offset) => *offset,
            None => self.allocate_new_page()?,
        };

        self.db_file_handle.seek(SeekFrom::Start(offset as u64))?;
        self.db_file_handle.read_exact(data)?;

        self.pages.insert(page_id, offset);

        Ok(())
    }

    pub fn write_page(&mut self, page_id: usize, data: &[u8]) -> Result<(), DiskManagerError> {
        if data.len() != DEFAULT_PAGE_SIZE {
            return Err(DiskManagerError::InvalidPageSize);
        }

        let offset = match self.pages.get(&page_id) {
            Some(offset) => *offset,
            None => self.allocate_new_page()?,
        };

        self.db_file_handle.seek(SeekFrom::Start(offset as u64))?;
        self.db_file_handle.write_all(data)?;

        self.pages.insert(page_id, offset);

        // std::fs::File isn't buffered, but we flush anyway just in case
        self.db_file_handle.flush()?;

        Ok(())
    }

    // Returns the offset within the file for the new page, gives away
    // previously deleted pages first before using fresh pages. Makes more
    // space in the file if necessary.
    fn allocate_new_page(&mut self) -> Result<usize, DiskManagerError> {
        if let Some(offset) = self.free_slots.pop() {
            return Ok(offset);
        }

        if self.pages.len() >= self.page_capacity {
            self.page_capacity *= 2;

            self.db_file_handle
                .set_len(((self.page_capacity + 1) * DEFAULT_PAGE_SIZE) as u64)?;
        }

        Ok(self.pages.len() * DEFAULT_PAGE_SIZE)
    }
}
