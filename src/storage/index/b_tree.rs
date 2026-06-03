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
            b_tree_leaf_page::{BTreeLeafPage, BTreeLeafPageMut},
            b_tree_node_header::BTreeNodeHeader,
            b_tree_root_page::{BTreeRootPage, BTreeRootPageMut},
        },
        rid::Rid,
    },
};

// We refer to it as a BTree for short, but in reality it's a B+ Tree
struct BTreeContext<'a> {
    header: Option<WritePageGuard<'a>>,
    root_page_id: PageId,
    write_set: Vec<WritePageGuard<'a>>,
    read_set: Vec<ReadPageGuard<'a>>,
}

impl<'a> BTreeContext<'a> {
    pub fn new() -> Self {
        Self {
            header: None,
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
        self.header.take();
    }

    pub fn release_read_ancestors(&mut self) {
        let current = self.read_set.pop();
        self.read_set.clear();
        if let Some(current) = current {
            self.read_set.push(current);
        }
        self.header.take();
    }

    pub fn release_write_ancestors(&mut self) {
        let current = self.write_set.pop();
        self.write_set.clear();
        if let Some(current) = current {
            self.write_set.push(current);
        }
        self.header.take();
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

    fn header_guard_mut(&self) -> Result<WritePageGuard<'_>, BTreeError> {
        self.get_write_guard(self.header_page_id)
    }

    fn get_read_guard(&self, page_id: PageId) -> Result<ReadPageGuard<'_>, BTreeError> {
        Ok(self.bpm.read_page(page_id)?)
    }

    fn get_write_guard(&self, page_id: PageId) -> Result<WritePageGuard<'_>, BTreeError> {
        Ok(self.bpm.write_page(page_id)?)
    }

    fn internal_page<'page>(data: &'page PageBytes) -> BTreeInternalPage<'page, K> {
        BTreeInternalPage::from_data(data)
    }

    fn leaf_page<'page>(data: &'page PageBytes) -> BTreeLeafPage<'page, K, TOMB_CAP> {
        BTreeLeafPage::from_data(data)
    }

    fn leaf_page_mut<'page>(data: &'page mut PageBytes) -> BTreeLeafPageMut<'page, K, TOMB_CAP> {
        BTreeLeafPageMut::from_data(data)
    }

    fn init_leaf_page<'page>(data: &'page mut PageBytes) -> BTreeLeafPageMut<'page, K, TOMB_CAP> {
        BTreeLeafPageMut::init(data)
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

        self.descend_read_path_for_insert(&mut ctx, key, value)?;

        let mut leaf_guard = ctx.write_set.pop().expect("cannot be empty");
        let mut leaf = Self::leaf_page_mut(leaf_guard.data_mut());
        let insert_pos = leaf.find_insert_pos(&key, &value, self.comparator);

        // See if key-value already exists in tree
        if insert_pos < leaf.curr_size()
            && leaf
                .cmp_key_rid_to_idx(&key, &value, insert_pos, self.comparator)
                .is_eq()
        {
            if leaf.is_idx_tombstoned(insert_pos) {
                leaf.remove_tombstone_at(insert_pos);
                return Ok(());
            } else {
                return Err(BTreeError::Duplicate);
            }
        }

        if !leaf.is_insert_safe() {
            drop(leaf_guard);
            ctx.release_read_path();
            return self.insert_pessimistic(key, value);
        }

        leaf.insert_at(insert_pos, &key, &value);

        Ok(())
    }

    fn insert_pessimistic(&self, key: K, value: Rid) -> Result<(), BTreeError> {
        let mut ctx = BTreeContext::new();
        let mut header_guard = self.header_guard_mut()?;
        // ctx.header = Some(self.header_guard_mut()?);
        let mut header_page = BTreeRootPageMut::from_data(header_guard.data_mut());

        if header_page.root_page_id() == INVALID_PAGE_ID {
            let new_page_id = self.bpm.new_page();
            header_page.set_root_page_id(new_page_id);

            // Could populate the context but it's kind of useless
            let mut new_root_guard = self.bpm.write_page(new_page_id)?;
            let mut new_root = Self::init_leaf_page(new_root_guard.data_mut());
            new_root.insert_at(0, &key, &value);

            return Ok(());
        }

        ctx.root_page_id = header_page.root_page_id();
        ctx.header = Some(header_guard);
        ctx.write_set.push(self.bpm.write_page(ctx.root_page_id)?);

        self.descend_write_path_for_insert(&mut ctx, key, value)?;

        let mut leaf_page = Self::leaf_page_mut(ctx.write_set.last_mut().unwrap().data_mut());
        let insert_idx = leaf_page.find_insert_pos(&key, &value, self.comparator);

        // key value already exists
        if insert_idx < leaf_page.curr_size()
            && leaf_page
                .cmp_key_rid_to_idx(&key, &value, insert_idx, self.comparator)
                .is_eq()
        {
            if leaf_page.is_idx_tombstoned(insert_idx) {
                leaf_page.remove_tombstone_at(insert_idx);
                return Ok(());
            }

            // Should not happen as RID's are unique, we wouldn't try to insert
            // it twice to an index
            return Err(BTreeError::Duplicate);
        }

        leaf_page.insert_at(insert_idx, &key, &value);

        // We don't want until we are full to split, immediately split when we
        // hit max capacity to simplify logic
        if leaf_page.curr_size() == leaf_page.max_size() {
            let sibling_page_id = self.bpm.new_page();
            let mut sibling_guard = self.bpm.write_page(sibling_page_id)?;
            let mut sibling_leaf = Self::init_leaf_page(sibling_guard.data_mut());
            sibling_leaf.set_next_page_id(leaf_page.get_next_page_id());

            let split_idx = leaf_page.min_size();
            leaf_page.move_split_entries_to(&mut sibling_leaf, split_idx);
            leaf_page.set_next_page_id(sibling_page_id);

            let separator_key = *sibling_leaf.key_ref(0);
            let separator_rid = *sibling_leaf.rid_ref(0);
            self.insert_into_parent(&mut ctx, &separator_key, &separator_rid, sibling_page_id);
        }

        Ok(())
    }

    // Crab latching until we reach the leaf page in which we should insert the key-value
    // Optimistically grab read latches along the way and get the write latch for the target
    // leaf page.
    fn descend_read_path_for_insert(
        &self,
        ctx: &mut BTreeContext<'a>,
        key: K,
        value: Rid,
    ) -> Result<(), BTreeError> {
        if BTreeNodeHeader::from_data(ctx.read_set.last().unwrap().data()).is_leaf() {
            let leaf_page_id = ctx.read_set.last().unwrap().page_id();
            ctx.read_set.pop();
            ctx.write_set.push(self.bpm.write_page(leaf_page_id)?);
            return Ok(());
        }

        // TODO: not very efficient to get the read latch on the leaf page
        // only to immediately swap it for a write latch. It would be smarter
        // to use the height of the BTree to anticipate whether or not the next
        // page is a leaf and immediately get a write latch for it.
        loop {
            let internal = Self::internal_page(ctx.read_set.last().unwrap().data());
            let next_page_id = *internal.value_at(internal.find_child_idx_for_insert(
                &key,
                &value,
                self.comparator,
            ));
            let next_guard = self.bpm.read_page(next_page_id)?;
            let next_page = BTreeNodeHeader::from_data(next_guard.data());

            if next_page.is_leaf() {
                // Drop so can get the writer latch instead
                drop(next_guard);
                ctx.write_set.push(self.bpm.write_page(next_page_id)?);
                return Ok(());
            }

            let insert_is_safe = next_page.is_insert_safe();
            ctx.read_set.push(next_guard);

            if insert_is_safe {
                ctx.release_read_ancestors();
            }
        }
    }

    fn descend_write_path_for_insert(
        &self,
        ctx: &mut BTreeContext<'a>,
        key: K,
        value: Rid,
    ) -> Result<(), BTreeError> {
        todo!()
    }

    fn insert_into_parent(
        &self,
        ctx: &mut BTreeContext,
        split_key: &K,
        split_value: &Rid,
        right_sibling_id: PageId,
    ) -> Result<(), BTreeError> {
        todo!()
    }
}
