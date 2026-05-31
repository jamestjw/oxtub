use crate::{
    buffer::{
        page::INVALID_PAGE_ID,
        page_guard::{ReadPageGuard, WritePageGuard},
    },
    common::types::PageId,
};

// We refer to it as a BTree for short, but in reality it's a B+ Tree
struct BTreeContext<'a> {
    header_page: Option<WritePageGuard<'a>>,
    root_page_id: PageId,
    write_set: Vec<WritePageGuard<'a>>,
    read_set: Vec<ReadPageGuard<'a>>,
}

impl<'a> BTreeContext<'a> {
    pub fn new() -> Self {
        Self {
            header_page: None,
            root_page_id: INVALID_PAGE_ID,
            write_set: vec![],
            read_set: vec![],
        }
    }
}
