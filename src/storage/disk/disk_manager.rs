use crate::common::types::PageId;
use crate::storage::disk::config::DEFAULT_PAGE_SIZE;
use crate::storage::disk::error::DiskManagerError;

use super::config::DEFAULT_DB_FILE_PAGE_CAPACITY;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

#[derive(Debug)]
pub struct DiskManager {
    db_file: PathBuf,
    db_file_handle: File,
    page_capacity: usize,
    pages: HashMap<PageId, usize>, // page_id -> file offset
    free_slots: Vec<usize>,        // free file offsets (from deleted pages)
}

impl DiskManager {
    pub fn new(db_file: PathBuf) -> Result<DiskManager, DiskManagerError> {
        let db_file_handle = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
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
    pub fn read_page(&mut self, page_id: PageId, data: &mut [u8]) -> Result<(), DiskManagerError> {
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

    pub fn write_page(&mut self, page_id: PageId, data: &[u8]) -> Result<(), DiskManagerError> {
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

    pub fn delete_page(&mut self, page_id: PageId) -> Result<(), DiskManagerError> {
        match self.pages.remove(&page_id) {
            None => Err(DiskManagerError::PageNotFound(page_id)),
            Some(offset) => {
                self.free_slots.push(offset);
                Ok(())
            }
        }
    }

    pub fn get_db_file_size(&self) -> Result<u64, DiskManagerError> {
        Ok(self.db_file_handle.metadata()?.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn setup_disk_manager() -> DiskManager {
        let file = NamedTempFile::new().unwrap();
        DiskManager::new(file.path().to_path_buf()).unwrap()
    }

    #[test]
    fn invalid_file() {
        let result = DiskManager::new(PathBuf::from("dev/null\\/foo/bar/baz/test"));
        assert!(matches!(result, Err(DiskManagerError::Io(_))));
    }

    #[test]
    fn read_inexistant_page() {
        let mut dm = setup_disk_manager();
        let mut buf = [0; DEFAULT_PAGE_SIZE];
        let result = dm.read_page(0, &mut buf);

        assert!(result.is_ok());
    }

    #[test]
    fn read_write_page() {
        let mut dm = setup_disk_manager();
        let mut buf = [0; DEFAULT_PAGE_SIZE];
        let mut output_buf = [0; DEFAULT_PAGE_SIZE];
        let test_string = "Hello world";
        let bytes = test_string.as_bytes();
        buf[..bytes.len()].copy_from_slice(bytes);

        let result = dm.write_page(0, &buf);
        assert!(result.is_ok());

        let result = dm.read_page(0, &mut output_buf);
        assert!(result.is_ok());

        assert_eq!(&buf[..], &output_buf[..]);
    }

    #[test]
    fn delete_page_test() -> Result<(), DiskManagerError> {
        let mut dm = setup_disk_manager();

        let initial_size = dm.get_db_file_size()?;
        let mut buf = [0u8; DEFAULT_PAGE_SIZE];
        let mut data = [0u8; DEFAULT_PAGE_SIZE];

        let text = "A test string.";
        data[..text.len()].copy_from_slice(text.as_bytes());

        let mut pages_to_write = 100;
        for page_id in 0..pages_to_write {
            dm.write_page(page_id, &data)?;
            dm.read_page(page_id, &mut buf)?;
            assert_eq!(buf, data);
        }

        let size_after_write = dm.get_db_file_size()?;
        assert!(size_after_write >= initial_size);

        pages_to_write *= 2;
        data = [0u8; DEFAULT_PAGE_SIZE];
        let text = "test string version 2";
        data[..text.len()].copy_from_slice(text.as_bytes());

        for page_id in 0..pages_to_write {
            dm.write_page(page_id, &data)?;
            dm.read_page(page_id, &mut buf)?;
            assert_eq!(buf, data);
            dm.delete_page(page_id)?;
        }
        let size_after_delete = dm.get_db_file_size()?;

        // Deleting pages just marks them as free, we don't reclaim anything
        assert_eq!(size_after_delete, size_after_write);
        Ok(())
    }
}
