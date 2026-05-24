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
