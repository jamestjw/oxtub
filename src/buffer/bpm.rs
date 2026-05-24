use std::{
    collections::HashMap,
    sync::{Mutex, RwLock},
};

use crate::{
    buffer::{frame::{Frame, FrameMeta}, page::Page, replacer::Replacer},
    storage::disk::disk_manager::DiskManager,
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
    disk: Mutex<DiskManager>,
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
}
