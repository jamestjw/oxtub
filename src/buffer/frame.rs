use std::sync::RwLock;

use super::page::Page;

pub(crate) struct Frame {
    pub(crate) page: RwLock<Page>,
}

pub(crate) struct FrameMeta {
    pub(crate) page_id: Option<usize>,
    pub(crate) pin_count: usize,
    pub(crate) is_dirty: bool,
}

impl FrameMeta {
    pub fn reset(&mut self) {
        self.page_id = None;
        self.pin_count = 0;
        self.is_dirty = false;
    }
}
