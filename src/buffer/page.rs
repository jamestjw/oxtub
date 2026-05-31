use crate::{common::types::PageId, storage::disk::config::DEFAULT_PAGE_SIZE};

pub const INVALID_PAGE_ID: PageId = 0;
pub type PageBytes = [u8; DEFAULT_PAGE_SIZE];

#[repr(align(8))]
pub struct PageData(pub PageBytes);

pub struct Page {
    page_id: Option<PageId>,
    data: PageData,
}

impl Page {
    pub fn new() -> Self {
        Self {
            page_id: None,
            data: PageData([0; DEFAULT_PAGE_SIZE]),
        }
    }

    pub fn data(&self) -> &PageBytes {
        &self.data.0
    }
    pub fn data_mut(&mut self) -> &mut PageBytes {
        &mut self.data.0
    }
    pub fn page_id(&self) -> Option<PageId> {
        self.page_id
    }
    pub fn set_page_id(&mut self, page_id: Option<PageId>) {
        self.page_id = page_id;
    }
}
