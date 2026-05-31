use std::{
    collections::HashMap,
    sync::{Mutex, RwLockWriteGuard},
};

use crate::{
    buffer::{
        frame::{Frame, FrameMeta},
        page::{INVALID_PAGE_ID, Page},
        page_guard::{ReadPageGuard, WritePageGuard},
        replacer::{LruKReplacer, Replacer},
    },
    common::types::PageId,
    storage::disk::{
        config::DEFAULT_PAGE_SIZE,
        disk_manager::DiskManager,
        disk_scheduler::{DiskScheduler, PageBuffer},
        error::BufferPoolError,
    },
};

pub struct BufferPoolManager {
    // TODO: see if this needs to be Arc
    inner: BufferPoolInner,
}

pub struct BufferPoolInner {
    pub(crate) state: Mutex<BufferPoolState>,
    // TODO: eventually, our buffer pool should just allocate a huge chunk
    // of memory at startup and just distribute that between the frames.
    frames: Vec<Frame>,
    disk_scheduler: DiskScheduler,
}

pub(crate) struct BufferPoolState {
    // page_id -> frame_id
    page_table: HashMap<PageId, usize>,
    pub(crate) frame_metas: Vec<FrameMeta>,
    // frame_ids that are free
    free_list: Vec<usize>,
    pub(crate) replacer: Box<dyn Replacer>,
    next_page_id: PageId,
}

impl BufferPoolManager {
    pub fn new(pool_size: usize, disk_manager: DiskManager) -> Self {
        let frames = (0..pool_size)
            .map(|_| Frame {
                page: std::sync::RwLock::new(Page::new()),
            })
            .collect();
        let frame_metas = (0..pool_size)
            .map(|_| FrameMeta {
                page_id: None,
                pin_count: 0,
                is_dirty: false,
            })
            .collect();

        Self {
            inner: BufferPoolInner {
                state: Mutex::new(BufferPoolState {
                    page_table: HashMap::new(),
                    frame_metas,
                    free_list: (0..pool_size).rev().collect(),
                    replacer: Box::new(LruKReplacer::new(pool_size, 2)),
                    next_page_id: INVALID_PAGE_ID + 1,
                }),
                frames,
                disk_scheduler: DiskScheduler::new(disk_manager),
            },
        }
    }

    // Number of frames managed by the BPM
    pub fn size(&self) -> usize {
        self.inner.frames.len()
    }

    pub fn new_page(&self) -> PageId {
        let mut state = self.inner.state.lock().unwrap();
        let page_id = state.next_page_id;
        state.next_page_id += 1;
        page_id
    }

    pub fn pin_count(&self, page_id: PageId) -> Option<usize> {
        let state = self.inner.state.lock().unwrap();
        state
            .page_table
            .get(&page_id)
            .map(|&frame_id| state.frame_metas[frame_id].pin_count)
    }

    // If the page is pinned in the buffer pool, this function does nothing and
    // returns `false`. Otherwise, this function removes the page from both disk
    // and memory (if it is still in the buffer pool), returning `true`.
    pub fn delete_page(&self, page_id: PageId) -> Result<(), BufferPoolError> {
        let mut state = self.inner.state.lock().unwrap();

        match state.page_table.get(&page_id).copied() {
            None => {
                tracing::warn!(page_id, "page not loaded in buffer pool");
            }
            Some(frame_id) => {
                if state.frame_metas[frame_id].pin_count > 0 {
                    return Err(BufferPoolError::PagePinned(page_id));
                }

                state.replacer.remove(frame_id);
                state.free_list.push(frame_id);
                state.page_table.remove(&page_id);
                state.frame_metas[frame_id].reset();
            }
        }

        // Don't hold the mutex while doing disk I/O
        drop(state);

        // If the page doesn't exist or the scheduler doesn't successfully delete the
        // page, it fails with a log. For now, this doesn't stop the DB's normal operations
        if let Err(e) = self.inner.disk_scheduler.delete_page(page_id) {
            // note: this error means that the request wasn't successfully sent, we don't check
            // if the scheduler managed to delete the page
            tracing::warn!(page_id, error = %e,  "could not send delete page req to scheduler");
        }

        Ok(())
    }

    /**
     * Acquires an optional write-locked guard over a page of data.
     *
     * Page data can _only_ be accessed via page guards. Users of this `BufferPoolManager`
     * are expected to acquire either a `ReadPageGuard` or a `WritePageGuard` depending on the mode
     * in which they would like to access the data, which ensures that any access of data is thread-safe.
     *
     * There can only be 1 `WritePageGuard` reading/writing a page at a time. This allows data access to
     * be both immutable and mutable, meaning the thread that owns the `WritePageGuard` is allowed to
     * manipulate the page's data however they want. If a user wants to have multiple threads reading
     * the page at the same time, they must acquire a `ReadPageGuard` with `CheckedReadPage` instead.
     *
     * Cases:
     * - The page has already been loaded into a frame, so we can just return a guard for it
     * - Buffer pool has plenty of empty frames, so just load the desired page into the frame and
     *   return a guard for it
     * - All frames are occupied, so we need to try to evict something using the replacement
     *   algorithm, then we load the page into the now free frame and return a guard for it.
     */
    pub fn write_page(&self, page_id: PageId) -> Result<WritePageGuard<'_>, BufferPoolError> {
        let mut state = self.inner.state.lock().unwrap();

        // Try to get a frame that we have already loaded the page into
        if let Some(frame_id) = Self::pin_loaded_page(&mut state, page_id) {
            // Page is loaded in a frame, this can mean two things:
            // - no one is using it right now, and it just hasn't been evicted yet
            // - someone is reading/writing to it, so when we try to get the read-write lock
            //   this may block

            // While we get the read-write lock, we may block. We might need to give up the latch
            // for the buffer pool state, otherwise the other threads that are holding on the
            // read-write lock will not be able to release it.
            drop(state);

            let write_guard = self.inner.frames[frame_id].page.write().unwrap();

            return Ok(WritePageGuard::new(
                &self.inner,
                frame_id,
                page_id,
                write_guard,
            ));
        }

        let (frame_id, mut write_guard) = self.acquire_frame_for_page_load(&mut state)?;

        // TODO: might have to rollback state correctly if this fails, though failure
        // should never really occur
        let buffer = self.inner.disk_scheduler.read_page(page_id)?;
        write_guard.data_mut().copy_from_slice(buffer.data());
        write_guard.set_page_id(Some(page_id));

        Self::publish_loaded_page(&mut state, page_id, frame_id);

        Ok(WritePageGuard::new(
            &self.inner,
            frame_id,
            page_id,
            write_guard,
        ))
    }

    pub fn read_page(&self, page_id: PageId) -> Result<ReadPageGuard<'_>, BufferPoolError> {
        let mut state = self.inner.state.lock().unwrap();

        // Try to get a frame that we have already loaded the page into
        if let Some(frame_id) = Self::pin_loaded_page(&mut state, page_id) {
            drop(state);

            let read_guard = self.inner.frames[frame_id].page.read().unwrap();

            return Ok(ReadPageGuard::new(
                &self.inner,
                frame_id,
                page_id,
                read_guard,
            ));
        }

        let (frame_id, mut write_guard) = self.acquire_frame_for_page_load(&mut state)?;

        // TODO: might have to rollback state correctly if this fails, though failure
        // should never really occur
        let buffer = self.inner.disk_scheduler.read_page(page_id)?;
        write_guard.data_mut().copy_from_slice(buffer.data());
        write_guard.set_page_id(Some(page_id));

        Self::publish_loaded_page(&mut state, page_id, frame_id);

        drop(write_guard);
        let read_guard = self.inner.frames[frame_id].page.read().unwrap();

        Ok(ReadPageGuard::new(
            &self.inner,
            frame_id,
            page_id,
            read_guard,
        ))
    }

    fn pin_loaded_page(state: &mut BufferPoolState, page_id: PageId) -> Option<usize> {
        match state.page_table.get(&page_id).copied() {
            Some(frame_id) => {
                state.frame_metas[frame_id].pin_count += 1;
                state.replacer.record_access(frame_id, page_id);
                state.replacer.set_evictable(frame_id, false);

                Some(frame_id)
            }
            None => None,
        }
    }

    fn publish_loaded_page(state: &mut BufferPoolState, page_id: PageId, frame_id: usize) {
        state.page_table.insert(page_id, frame_id);
        state.frame_metas[frame_id].pin_count = 1;
        state.frame_metas[frame_id].is_dirty = false;
        state.frame_metas[frame_id].page_id = Some(page_id);
        state.replacer.record_access(frame_id, page_id);
        state.replacer.set_evictable(frame_id, false);
    }

    fn acquire_frame_for_page_load(
        &self,
        state: &mut BufferPoolState,
    ) -> Result<(usize, RwLockWriteGuard<'_, Page>), BufferPoolError> {
        // Try to get a free frame to load the page into
        match state.free_list.pop() {
            Some(frame_id) => Ok((frame_id, self.inner.frames[frame_id].page.write().unwrap())),
            // No free frame available, try to evict a page
            None => {
                let frame_id = state
                    .replacer
                    .evict()
                    .ok_or(BufferPoolError::NoAvailableFrame)?;
                let victim_page_id = state.frame_metas[frame_id]
                    .page_id
                    .expect("frame has no page id");
                let write_guard = self.inner.frames[frame_id].page.write().unwrap();

                // need to flush this page to disk before we re-use its frame
                if state.frame_metas[frame_id].is_dirty {
                    let mut data_arr = [0u8; DEFAULT_PAGE_SIZE];
                    data_arr.copy_from_slice(write_guard.data());
                    // TODO: a smart optimisation to increase buffer pool throughput
                    // is to 'reserve' the frame and remove the victim page from the page
                    // table, give up the BPM mutex, do the required I/O and then re-acquire
                    // the lock. The key idea is that once the frame is required, even if we
                    // give up the mutex, no one else will be able to 'take it' from us.
                    if let Err(err) = self
                        .inner
                        .disk_scheduler
                        .write_page(victim_page_id, PageBuffer::of_data(data_arr))
                    {
                        // Roll back replacer state removed by evict
                        // TODO: this doesn't put things back in the exact state as things
                        // were before because of the replacer's internal algorithm
                        state.replacer.record_access(frame_id, victim_page_id);
                        state.replacer.set_evictable(frame_id, true);
                        return Err(err.into());
                    }
                }

                state.page_table.remove(&victim_page_id);
                state.frame_metas[frame_id].reset();

                Ok((frame_id, write_guard))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc, Condvar, Mutex as StdMutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        thread,
        time::Duration,
    };

    use tempfile::NamedTempFile;

    use super::*;

    const FRAMES: usize = 10;

    fn setup_bpm(pool_size: usize) -> BufferPoolManager {
        let file = NamedTempFile::new().unwrap();
        let disk_manager = DiskManager::new(file.path().to_path_buf()).unwrap();
        BufferPoolManager::new(pool_size, disk_manager)
    }

    fn copy_string(dest: &mut [u8], src: &str) {
        assert!(src.len() + 1 <= dest.len());
        dest.fill(0);
        dest[..src.len()].copy_from_slice(src.as_bytes());
    }

    fn read_string(data: &[u8]) -> &str {
        let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
        std::str::from_utf8(&data[..end]).unwrap()
    }

    #[test]
    fn very_basic_test() {
        let bpm = setup_bpm(FRAMES);

        let pid = bpm.new_page();
        let text = "Hello, world!";

        {
            let mut guard = bpm.write_page(pid).unwrap();
            copy_string(guard.data_mut(), text);
            assert_eq!(read_string(guard.data()), text);
        }

        {
            let guard = bpm.read_page(pid).unwrap();
            assert_eq!(read_string(guard.data()), text);
        }

        {
            let guard = bpm.read_page(pid).unwrap();
            assert_eq!(read_string(guard.data()), text);
        }

        bpm.delete_page(pid).unwrap();
    }

    #[test]
    fn page_pin_easy_test() {
        let bpm = setup_bpm(2);

        let page_id0 = bpm.new_page();
        let page_id1 = bpm.new_page();

        {
            let mut page0_write = bpm.write_page(page_id0).unwrap();
            copy_string(page0_write.data_mut(), "page0");

            let mut page1_write = bpm.write_page(page_id1).unwrap();
            copy_string(page1_write.data_mut(), "page1");

            assert_eq!(bpm.pin_count(page_id0), Some(1));
            assert_eq!(bpm.pin_count(page_id1), Some(1));

            let temp_page_id1 = bpm.new_page();
            assert!(matches!(
                bpm.read_page(temp_page_id1),
                Err(BufferPoolError::NoAvailableFrame)
            ));

            let temp_page_id2 = bpm.new_page();
            assert!(matches!(
                bpm.write_page(temp_page_id2),
                Err(BufferPoolError::NoAvailableFrame)
            ));

            assert_eq!(bpm.pin_count(page_id0), Some(1));
            drop(page0_write);
            assert_eq!(bpm.pin_count(page_id0), Some(0));

            assert_eq!(bpm.pin_count(page_id1), Some(1));
            drop(page1_write);
            assert_eq!(bpm.pin_count(page_id1), Some(0));
        }

        {
            let temp_page_id1 = bpm.new_page();
            assert!(bpm.read_page(temp_page_id1).is_ok());

            let temp_page_id2 = bpm.new_page();
            assert!(bpm.write_page(temp_page_id2).is_ok());

            assert_eq!(bpm.pin_count(page_id0), None);
            assert_eq!(bpm.pin_count(page_id1), None);
        }

        {
            let mut page0_write = bpm.write_page(page_id0).unwrap();
            assert_eq!(read_string(page0_write.data()), "page0");
            copy_string(page0_write.data_mut(), "page0updated");

            let mut page1_write = bpm.write_page(page_id1).unwrap();
            assert_eq!(read_string(page1_write.data()), "page1");
            copy_string(page1_write.data_mut(), "page1updated");

            assert_eq!(bpm.pin_count(page_id0), Some(1));
            assert_eq!(bpm.pin_count(page_id1), Some(1));
        }

        assert_eq!(bpm.pin_count(page_id0), Some(0));
        assert_eq!(bpm.pin_count(page_id1), Some(0));

        {
            let page0_read = bpm.read_page(page_id0).unwrap();
            assert_eq!(read_string(page0_read.data()), "page0updated");

            let page1_read = bpm.read_page(page_id1).unwrap();
            assert_eq!(read_string(page1_read.data()), "page1updated");

            assert_eq!(bpm.pin_count(page_id0), Some(1));
            assert_eq!(bpm.pin_count(page_id1), Some(1));
        }

        assert_eq!(bpm.pin_count(page_id0), Some(0));
        assert_eq!(bpm.pin_count(page_id1), Some(0));
    }

    #[test]
    fn page_pin_medium_test() {
        let bpm = setup_bpm(FRAMES);

        let pid0 = bpm.new_page();
        let mut page0 = bpm.write_page(pid0).unwrap();

        copy_string(page0.data_mut(), "Hello");
        assert_eq!(read_string(page0.data()), "Hello");
        drop(page0);

        let mut pages = Vec::new();
        for _ in 0..FRAMES {
            let pid = bpm.new_page();
            pages.push(bpm.write_page(pid).unwrap());
        }

        for page in &pages {
            assert_eq!(bpm.pin_count(page.page_id()), Some(1));
        }

        // checking that multiple failures don't corrupt the bpm's state
        for _ in 0..FRAMES {
            let pid = bpm.new_page();
            assert!(matches!(
                bpm.write_page(pid),
                Err(BufferPoolError::NoAvailableFrame)
            ));
        }

        for _ in 0..FRAMES / 2 {
            let pid = pages[0].page_id();
            assert_eq!(bpm.pin_count(pid), Some(1));
            pages.remove(0);
            assert_eq!(bpm.pin_count(pid), Some(0));
        }

        for page in &pages {
            assert_eq!(bpm.pin_count(page.page_id()), Some(1));
        }

        for _ in 0..((FRAMES / 2) - 1) {
            let pid = bpm.new_page();
            pages.push(bpm.write_page(pid).unwrap());
        }

        {
            let original_page = bpm.read_page(pid0).unwrap();
            assert_eq!(read_string(original_page.data()), "Hello");
        }

        let last_pid = bpm.new_page();
        let _last_page = bpm.read_page(last_pid).unwrap();
        assert!(matches!(
            bpm.read_page(pid0),
            Err(BufferPoolError::NoAvailableFrame)
        ));
    }

    #[test]
    fn page_access_test() {
        let rounds = 50;
        let bpm = Arc::new(setup_bpm(1));
        let pid = bpm.new_page();

        let writer_bpm = Arc::clone(&bpm);
        let writer = thread::spawn(move || {
            for i in 0..rounds {
                thread::sleep(Duration::from_millis(5));
                let mut guard = writer_bpm.write_page(pid).unwrap();
                copy_string(guard.data_mut(), &i.to_string());
            }
        });

        // Verify that while a read lock is held, no writers can touch the
        // page
        for _ in 0..rounds {
            thread::sleep(Duration::from_millis(10));
            let guard = bpm.read_page(pid).unwrap();
            let observed = *guard.data();
            thread::sleep(Duration::from_millis(10));
            assert_eq!(guard.data(), &observed);
        }

        writer.join().unwrap();
    }

    #[test]
    fn contention_test() {
        // Repeated concurrent writes to one page should serialize without
        // deadlock or pin-count corruption.
        let bpm = Arc::new(setup_bpm(FRAMES));
        let rounds = 10_000;
        let pid = bpm.new_page();

        let handles: Vec<_> = (0..4)
            .map(|_| {
                let bpm = Arc::clone(&bpm);
                thread::spawn(move || {
                    for i in 0..rounds {
                        let mut guard = bpm.write_page(pid).unwrap();
                        copy_string(guard.data_mut(), &i.to_string());
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        {
            let guard = bpm.read_page(pid).unwrap();
            assert_eq!(read_string(guard.data()), (rounds - 1).to_string());
        }

        assert_eq!(bpm.pin_count(pid), Some(0));
    }

    #[test]
    fn deadlock_test() {
        // Waiting on one page latch must not keep the global BPM
        // mutex and block access to another page.
        let bpm = Arc::new(setup_bpm(FRAMES));
        let pid0 = bpm.new_page();
        let pid1 = bpm.new_page();

        let guard0 = bpm.write_page(pid0).unwrap();
        let start = Arc::new(AtomicBool::new(false));

        let child_bpm = Arc::clone(&bpm);
        let child_start = Arc::clone(&start);
        let child = thread::spawn(move || {
            child_start.store(true, Ordering::SeqCst);
            let _guard0 = child_bpm.write_page(pid0).unwrap();
        });

        while !start.load(Ordering::SeqCst) {
            std::hint::spin_loop();
        }

        thread::sleep(Duration::from_millis(100));
        let _guard1 = bpm.write_page(pid1).unwrap();
        drop(guard0);
        child.join().unwrap();
    }

    #[test]
    fn reader_then_writer_deadlock_test() {
        let bpm = Arc::new(setup_bpm(FRAMES));
        let pid0 = bpm.new_page();
        let pid1 = bpm.new_page();

        let num_readers = 4;
        let readers_ready = Arc::new(AtomicUsize::new(0));
        let release_readers = Arc::new(AtomicBool::new(false));
        let writer_acquired = Arc::new(AtomicBool::new(false));

        let mut readers = Vec::new();
        for _ in 0..num_readers {
            let bpm = Arc::clone(&bpm);
            let readers_ready = Arc::clone(&readers_ready);
            let release_readers = Arc::clone(&release_readers);
            readers.push(thread::spawn(move || {
                let _guard = bpm.read_page(pid0).unwrap();
                readers_ready.fetch_add(1, Ordering::SeqCst);
                while !release_readers.load(Ordering::SeqCst) {
                    std::hint::spin_loop();
                }
            }));
        }

        while readers_ready.load(Ordering::SeqCst) != num_readers {
            std::hint::spin_loop();
        }

        let writer_bpm = Arc::clone(&bpm);
        let writer_acquired_clone = Arc::clone(&writer_acquired);
        let writer = thread::spawn(move || {
            let _guard = writer_bpm.write_page(pid0).unwrap();
            writer_acquired_clone.store(true, Ordering::SeqCst);
        });

        {
            let _other_guard = bpm.write_page(pid1).unwrap();
        }

        release_readers.store(true, Ordering::SeqCst);

        for reader in readers {
            reader.join().unwrap();
        }
        writer.join().unwrap();

        assert!(writer_acquired.load(Ordering::SeqCst));
    }

    #[test]
    fn writer_then_reader_deadlock_test() {
        let bpm = Arc::new(setup_bpm(FRAMES));
        let pid0 = bpm.new_page();
        let pid1 = bpm.new_page();

        let writer_ready = Arc::new(AtomicBool::new(false));
        let release_writer = Arc::new(AtomicBool::new(false));
        let readers_acquired = Arc::new(AtomicUsize::new(0));

        let writer_bpm = Arc::clone(&bpm);
        let writer_ready_clone = Arc::clone(&writer_ready);
        let release_writer_clone = Arc::clone(&release_writer);
        let writer = thread::spawn(move || {
            let _guard = writer_bpm.write_page(pid0).unwrap();
            writer_ready_clone.store(true, Ordering::SeqCst);
            while !release_writer_clone.load(Ordering::SeqCst) {
                std::hint::spin_loop();
            }
        });

        while !writer_ready.load(Ordering::SeqCst) {
            std::hint::spin_loop();
        }

        let num_readers = 4;
        let mut readers = Vec::new();
        for _ in 0..num_readers {
            let bpm = Arc::clone(&bpm);
            let readers_acquired = Arc::clone(&readers_acquired);
            readers.push(thread::spawn(move || {
                let _guard = bpm.read_page(pid0).unwrap();
                readers_acquired.fetch_add(1, Ordering::SeqCst);
            }));
        }

        {
            let _other_guard = bpm.read_page(pid1).unwrap();
        }

        release_writer.store(true, Ordering::SeqCst);

        writer.join().unwrap();
        for reader in readers {
            reader.join().unwrap();
        }

        assert_eq!(readers_acquired.load(Ordering::SeqCst), num_readers);
    }

    #[test]
    fn evictable_test() {
        // A frame must remain non-evictable while any reader or writer
        // has the page pinned.
        let rounds = 100;
        let num_readers = 8;
        let bpm = Arc::new(setup_bpm(1));

        for i in 0..rounds {
            let pair = Arc::new((StdMutex::new(false), Condvar::new()));
            let winner_pid = bpm.new_page();
            let loser_pid = bpm.new_page();

            let mut readers = Vec::new();
            for _ in 0..num_readers {
                let bpm = Arc::clone(&bpm);
                let pair = Arc::clone(&pair);
                readers.push(thread::spawn(move || {
                    let (mutex, cv) = &*pair;
                    let mut signal = mutex.lock().unwrap();
                    while !*signal {
                        signal = cv.wait(signal).unwrap();
                    }
                    drop(signal);

                    let _read_guard = bpm.read_page(winner_pid).unwrap();
                    assert!(matches!(
                        bpm.read_page(loser_pid),
                        Err(BufferPoolError::NoAvailableFrame)
                    ));
                }));
            }

            let (mutex, cv) = &*pair;
            let mut signal = mutex.lock().unwrap();

            if i % 2 == 0 {
                let read_guard = bpm.read_page(winner_pid).unwrap();
                *signal = true;
                cv.notify_all();
                drop(signal);
                drop(read_guard);
            } else {
                let write_guard = bpm.write_page(winner_pid).unwrap();
                *signal = true;
                cv.notify_all();
                drop(signal);
                drop(write_guard);
            }

            for reader in readers {
                reader.join().unwrap();
            }
        }
    }
}
