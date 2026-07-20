use tempfile::NamedTempFile;

use crate::{buffer::bpm::BufferPoolManager, storage::disk::disk_manager::DiskManager};

pub(crate) fn setup_bpm(pool_size: usize) -> BufferPoolManager {
    let file = NamedTempFile::new().unwrap();
    let disk_manager = DiskManager::new(file.path().to_path_buf()).unwrap();
    BufferPoolManager::new(pool_size, disk_manager)
}
