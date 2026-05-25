use std::{
    collections::HashMap,
    sync::{Mutex, RwLockWriteGuard},
};

use crate::{
    buffer::{
        frame::{Frame, FrameMeta},
        page::Page,
        page_guard::WritePageGuard,
        replacer::Replacer,
    },
    storage::disk::{
        config::DEFAULT_PAGE_SIZE,
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
    page_table: HashMap<usize, usize>,
    pub(crate) frame_metas: Vec<FrameMeta>,
    // frame_ids that are free
    free_list: Vec<usize>,
    pub(crate) replacer: Box<dyn Replacer>,
    next_page_id: usize,
}

impl BufferPoolManager {
    // Number of frames managed by the BPM
    pub fn size(&self) -> usize {
        self.inner.frames.len()
    }

    pub fn new_page(&mut self) -> usize {
        let mut state = self.inner.state.lock().unwrap();
        let page_id = state.next_page_id;
        state.next_page_id += 1;
        page_id
    }

    // If the page is pinned in the buffer pool, this function does nothing and
    // returns `false`. Otherwise, this function removes the page from both disk
    // and memory (if it is still in the buffer pool), returning `true`.
    pub fn delete_page(&self, page_id: usize) -> Result<(), BufferPoolError> {
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
    pub fn write_page(&self, page_id: usize) -> Result<WritePageGuard<'_>, BufferPoolError> {
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

    fn pin_loaded_page(state: &mut BufferPoolState, page_id: usize) -> Option<usize> {
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

    fn publish_loaded_page(state: &mut BufferPoolState, page_id: usize, frame_id: usize) {
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
            None => match state.replacer.evict() {
                Some(frame_id) => {
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
                        self.inner
                            .disk_scheduler
                            .write_page(victim_page_id, PageBuffer::of_data(data_arr))?;
                    }

                    state.page_table.remove(&victim_page_id);
                    state.frame_metas[frame_id].reset();

                    Ok((frame_id, write_guard))
                }
                None => Err(BufferPoolError::NoAvailableFrame),
            },
        }
    }
}
