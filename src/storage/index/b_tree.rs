use std::marker::PhantomData;

use crate::{
    buffer::{
        bpm::BufferPoolManager,
        page::{INVALID_PAGE_ID, PageBytes},
        page_guard::{ReadPageGuard, WritePageGuard},
    },
    common::types::PageId,
    storage::{
        index::{
            comparator::{self, KeyComparator},
            error::BTreeError,
        },
        page::{
            b_tree_internal_page::BTreeInternalPage,
            b_tree_leaf_page::BTreeLeafPage,
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

    pub fn release_read_path(&mut self) {
        self.read_set.clear();
    }
}

// The BTree has a header page that holds the page_id of the actual
// root of the tree. This page will be alive as long as the BTree is
// alive. If the tree is empty, then the root_id will be INVALID_PAGE_ID.
pub struct BTree<'a, K, C, const TOMB_CAP: usize>
where
    C: KeyComparator<K>,
{
    bpm: &'a BufferPoolManager,
    header_page_id: PageId,
    comparator: &'a C,
    _marker: PhantomData<K>,
}

impl<'a, K: bytemuck::Pod + Copy, C: KeyComparator<K>, const TOMB_CAP: usize>
    BTree<'a, K, C, TOMB_CAP>
{
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

    fn header_guard(&self) -> Result<ReadPageGuard<'_>, BTreeError> {
        self.get_read_guard(self.header_page_id)
    }

    fn get_read_guard(&self, page_id: PageId) -> Result<ReadPageGuard<'_>, BTreeError> {
        Ok(self.bpm.read_page(page_id)?)
    }

    fn internal_page<'page>(data: &'page PageBytes) -> BTreeInternalPage<'page, K> {
        BTreeInternalPage::from_data(data)
    }

    fn leaf_page<'page>(data: &'page PageBytes) -> BTreeLeafPage<'page, K, TOMB_CAP> {
        BTreeLeafPage::from_data(data)
    }

    pub fn is_empty(&self) -> Result<bool, BTreeError> {
        let guard = self.header_guard()?;
        Ok(BTreeRootPage::from_data(guard.data()).root_page_id() == INVALID_PAGE_ID)
    }

    // SEARCH
    pub fn get_values(&self, key: &K) -> Result<Vec<Rid>, BTreeError> {
        let current_page_id = {
            let header_guard = self.header_guard()?;
            let root_page = BTreeRootPage::from_data(header_guard.data());

            if root_page.root_page_id() == INVALID_PAGE_ID {
                return Ok(vec![]);
            }

            root_page.root_page_id()
        };
        let mut current_guard = self.get_read_guard(current_page_id)?;

        while !BTreeNodeHeader::from_data(current_guard.data()).is_leaf() {
            let internal_page = Self::internal_page(current_guard.data());
            let next_page_id =
                internal_page.value_at(internal_page.find_child_idx(key, self.comparator));
            current_guard = self.get_read_guard(*next_page_id)?;
        }

        let mut idx = {
            let leaf = Self::leaf_page(current_guard.data());
            leaf.find_pos(key, self.comparator)
        };

        let mut result = vec![];

        loop {
            let next_page_id = {
                let leaf = Self::leaf_page(current_guard.data());

                while idx < leaf.curr_size() {
                    match self.comparator.compare(leaf.key_at(idx), key) {
                        std::cmp::Ordering::Less => idx += 1,
                        std::cmp::Ordering::Equal => {
                            if !leaf.is_idx_tombstoned(idx) {
                                result.push(*leaf.value_at(idx));
                            }
                            idx += 1;
                        }
                        std::cmp::Ordering::Greater => return Ok(result),
                    }
                }

                leaf.get_next_page_id()
            };

            if next_page_id == INVALID_PAGE_ID {
                return Ok(result);
            }

            current_guard = self.get_read_guard(next_page_id)?;
            idx = 0;
        }
    }

    // Insertion:
    //  if current tree is empty, start new tree, update root page id and insert
    //  entry; otherwise, insert into leaf page.
    pub fn insert(&self, key: K, value: Rid) -> Result<(), BTreeError> {
        let header_guard = self.header_guard()?;
        let current_page_id = BTreeRootPage::from_data(header_guard.data()).root_page_id();
        if current_page_id == INVALID_PAGE_ID {
            // Insert pessimistic will try to get write latches all the way down,
            // so we need to give up the read latch
            drop(header_guard);
            return self.insert_pessimistic(key, value);
        }

        let mut ctx = BTreeContext::new();
        ctx.read_set.push(header_guard);
        ctx.read_set.push(self.bpm.read_page(current_page_id)?);

        self.descend_read_path_for_insert(&mut ctx, key)?;

        let leaf = Self::leaf_page(ctx.read_set.last().expect("cannot be empty").data());
        let insert_pos = leaf.find_insert_pos(&key, &value, self.comparator);

        // If the key-value pair exists and is tombstoned, then this will be a safe insert
        let is_tombstoned = insert_pos < leaf.curr_size()
            && leaf
                .cmp_key_rid_to_idx(&key, &value, insert_pos, self.comparator)
                .is_eq()
            && leaf.is_idx_tombstoned(insert_pos);
        let insert_safe = is_tombstoned || leaf.is_insert_safe();

        // At this point, we have all the information we need, give up all read latches.
        // Now, we either know a fact that insertion is not 'safe', then we immediately
        // delegate to pessimistic insertion. Otherwise, we try to get a write latch on
        // the leaf. Then we see if things are still OK for us to proceed with our
        // optimistic write, if things look off, then we fall back to pessimistic insert.
        ctx.release_read_path();

        if !insert_safe {
            return self.insert_pessimistic(key, value);
        }

        todo!("finish the rest of logic and pessimistic insert");

        Ok(())
    }

    fn insert_pessimistic(&self, key: K, value: Rid) -> Result<(), BTreeError> {
        todo!()
    }

    fn descend_read_path_for_insert(
        &self,
        ctx: &mut BTreeContext,
        key: K,
    ) -> Result<(), BTreeError> {
        todo!()
    }
}
