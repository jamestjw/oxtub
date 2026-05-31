use std::marker::PhantomData;

use crate::{
    buffer::{
        bpm::BufferPoolManager,
        page::INVALID_PAGE_ID,
        page_guard::{ReadPageGuard, WritePageGuard},
    },
    common::types::PageId,
    storage::{
        index::comparator::{self, KeyComparator},
        page::{
            b_tree_internal_page::BTreeInternalPage,
            b_tree_node_header::BTreeNodeHeader,
            b_tree_root_page::{BTreeRootPage, BTreeRootPageMut},
        },
        rid::Rid,
    },
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

    pub fn is_root(&self, page_id: PageId) -> bool {
        page_id == self.root_page_id
    }

    pub fn release_all(&mut self) {
        self.read_set.clear();
        self.write_set.clear();
        self.header_page.take();
    }

    pub fn release_read_ancestors(&mut self) {
        let current = self.read_set.pop();
        self.read_set.clear();
        if let Some(current) = current {
            self.read_set.push(current);
        }
        self.header_page.take();
    }

    pub fn release_write_ancestors(&mut self) {
        let current = self.write_set.pop();
        self.write_set.clear();
        if let Some(current) = current {
            self.write_set.push(current);
        }
        self.header_page.take();
    }
}

// The BTree has a header page that holds the page_id of the actual
// root of the tree. This page will be alive as long as the BTree is
// alive. If the tree is empty, then the root_id will be INVALID_PAGE_ID.
pub struct BTree<'a, K, C>
where
    C: KeyComparator<K>,
{
    bpm: &'a BufferPoolManager,
    header_page_id: PageId,
    comparator: &'a C,
    _marker: PhantomData<K>,
}

impl<'a, K: bytemuck::Pod + Copy, C: KeyComparator<K>> BTree<'a, K, C> {
    pub fn new(bpm: &'a BufferPoolManager, header_page_id: PageId, comparator: &'a C) -> Self {
        let btree = Self {
            bpm,
            header_page_id,
            comparator,
            _marker: PhantomData,
        };

        let mut guard = bpm
            .write_page(header_page_id)
            .expect("unexpected buffer pool error");
        BTreeRootPageMut::from_data(guard.data_mut()).set_root_page_id(INVALID_PAGE_ID);

        btree
    }

    fn header_guard(&self) -> ReadPageGuard<'_> {
        self.get_read_guard(self.header_page_id)
    }

    fn get_read_guard(&self, page_id: PageId) -> ReadPageGuard<'_> {
        self.bpm
            .read_page(page_id)
            .expect("unexpected buffer pool error")
    }

    pub fn is_empty(&self) -> bool {
        BTreeRootPage::from_data(self.header_guard().data()).root_page_id() == INVALID_PAGE_ID
    }

    pub fn get_values(&self, key: &K) -> Vec<Rid> {
        let mut current_page_id = {
            let header_guard = self.header_guard();
            let root_page = BTreeRootPage::from_data(header_guard.data());

            assert!(
                root_page.root_page_id() != INVALID_PAGE_ID,
                "querying invalid btree"
            );
            root_page.root_page_id()
        };
        let mut current_guard = self.get_read_guard(current_page_id);

        while !BTreeNodeHeader::from_data(current_guard.data()).is_leaf() {
            let internal_page = BTreeInternalPage::from_data(current_guard.data());
        }

        // auto current_guard = bpm_->ReadPage(current_page_id);
        // while (!current_guard.As<BPlusTreePage>()->IsLeafPage()) {
        //   auto internal = current_guard.As<InternalPage>();
        //   page_id_t next_page_id = internal->ValueAt(FindChildInInternal(internal, key));
        //   auto next_guard = bpm_->ReadPage(next_page_id);
        //   current_guard.Drop();
        //   current_guard = std::move(next_guard);
        // }

        vec![]
    }
}
