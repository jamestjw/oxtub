use std::{collections::HashMap, sync::Mutex};

use crate::{
    buffer::{
        frame::{Frame, FrameMeta},
        replacer::Replacer,
    },
    storage::disk::disk_scheduler::DiskScheduler,
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
    pub fn delete_page(&self, page_id: usize) -> bool {
        let mut state = self.inner.state.lock().unwrap();

        match state.page_table.get(&page_id).copied() {
            None => {
                tracing::warn!(page_id, "page not loaded in buffer pool");
            }
            Some(frame_id) => {
                if state.frame_metas[frame_id].pin_count > 0 {
                    return false;
                }

                state.replacer.remove(frame_id);
                state.free_list.push(frame_id);
                state.page_table.remove(&page_id);
                state.frame_metas[frame_id].reset();
            }
        }

        // If the page doesn't exist or the scheduler doesn't successfully delete the
        // page, it fails with a log. For now, this doesn't stop the DB's normal operations
        if let Err(e) = self.inner.disk_scheduler.delete_page(page_id) {
            // note: this error means that the request wasn't successfully sent, we don't check
            // if the scheduler managed to delete the page
            tracing::warn!(page_id, error = %e,  "could not send delete page req to scheduler");
        }
        true
    }
}
