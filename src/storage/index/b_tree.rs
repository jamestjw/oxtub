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
            index_iterator::IndexIterator,
        },
        page::{
            b_tree_internal_page::{BTreeInternalPage, BTreeInternalPageMut},
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

    pub fn set_root_page_id(&mut self, page_id: PageId) {
        self.root_page_id = page_id;
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
        self.header.take();
        let current = self.write_set.pop();
        self.write_set.clear();
        if let Some(current) = current {
            self.write_set.push(current);
        }
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

    fn internal_page_mut<'page>(data: &'page mut PageBytes) -> BTreeInternalPageMut<'page, K> {
        BTreeInternalPageMut::from_data(data)
    }

    fn init_internal_page<'page>(data: &'page mut PageBytes) -> BTreeInternalPageMut<'page, K> {
        BTreeInternalPageMut::init(data)
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
            self.insert_into_parent(&mut ctx, &separator_key, &separator_rid, sibling_page_id)?;
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
        loop {
            let last_guard = ctx.write_set.last().unwrap();

            if BTreeNodeHeader::from_data(last_guard.data()).is_leaf() {
                return Ok(());
            }

            let internal_page = Self::internal_page(last_guard.data());
            let child_page_id = internal_page.value_at(internal_page.find_child_idx_for_insert(
                &key,
                &value,
                self.comparator,
            ));
            let child_guard = self.bpm.write_page(*child_page_id)?;
            let child_page = BTreeNodeHeader::from_data(child_guard.data());

            if child_page.is_insert_safe() {
                ctx.write_set.push(child_guard);
                ctx.release_write_ancestors();
            } else {
                ctx.write_set.push(child_guard);
            }
        }
    }

    fn insert_into_parent(
        &self,
        ctx: &mut BTreeContext,
        split_key: &K,
        split_value: &Rid,
        right_sibling_id: PageId,
    ) -> Result<(), BTreeError> {
        let mut key_to_insert = *split_key;
        let mut rid_to_insert = *split_value;
        let mut new_right_child_id = right_sibling_id;

        loop {
            let left_child_id = { ctx.write_set.pop().unwrap().page_id() };

            // Any retained safe ancestor would have absorbed the split before propagation
            // reached this point, so an empty write set means the split child was the root.
            if (ctx.write_set.is_empty()) {
                // If the left sibling itself is the root, then we create a new root that will
                // have the left and right siblings as children
                assert!(ctx.is_root(left_child_id));
                let new_root_page_id = self.bpm.new_page();
                let mut new_root_guard = self.bpm.write_page(new_root_page_id)?;
                let mut new_root = Self::init_internal_page(new_root_guard.data_mut());
                new_root.set_value_at(0, left_child_id);
                new_root.set_index_key_at(1, &key_to_insert, &rid_to_insert);
                new_root.set_value_at(1, new_right_child_id);
                new_root.set_size(2);

                let mut header_page =
                    BTreeRootPageMut::from_data(ctx.header.as_mut().unwrap().data_mut());
                header_page.set_root_page_id(new_root_page_id);
                ctx.root_page_id = new_root_page_id;

                return Ok(());
            }

            let mut parent = Self::internal_page_mut(ctx.write_set.last_mut().unwrap().data_mut());

            // Parent has room, just insert new child and we are done
            if parent.curr_size() < parent.max_size() {
                parent.insert_after(
                    &left_child_id,
                    key_to_insert,
                    rid_to_insert,
                    new_right_child_id,
                );
                return Ok(());
            }

            // Parent has no room, so we need to split the parent
            let parent_sibling_page_id = self.bpm.new_page();
            let mut parent_sibling_guard = self.bpm.write_page(parent_sibling_page_id)?;
            let mut parent_sibling = Self::init_internal_page(parent_sibling_guard.data_mut());
            let promoted_key = parent.split_insert_after(
                &mut parent_sibling,
                &left_child_id,
                key_to_insert,
                rid_to_insert,
                new_right_child_id,
            );

            // Keep looping, with the parent as the 'left sibling' (last element in the write set),
            // and it's new sibling which we explicitly pass as `new_right_child_id`
            (key_to_insert, rid_to_insert) = promoted_key;
            new_right_child_id = parent_sibling_page_id;
        }
    }

    pub fn remove(&self, key: K, value: Rid) -> Result<(), BTreeError> {
        let mut ctx = BTreeContext::new();

        let header_guard = self.header_guard()?;
        let header_page = BTreeRootPage::from_data(header_guard.data());
        let root_page_id = header_page.root_page_id();

        if root_page_id == INVALID_PAGE_ID {
            return Err(BTreeError::NotFound);
        }

        ctx.root_page_id = root_page_id;
        ctx.read_set.push(header_guard);
        ctx.read_set.push(self.bpm.read_page(root_page_id)?);

        self.descend_read_path_for_delete(&mut ctx, key)?;

        let mut leaf_guard = ctx.write_set.pop().unwrap();
        let leaf_page_id = leaf_guard.page_id();
        let mut leaf = Self::leaf_page_mut(leaf_guard.data_mut());

        // See if the leaf actually contains the key
        let idx = leaf.find_insert_pos(&key, &value, self.comparator);
        if idx >= leaf.curr_size()
            || leaf
                .cmp_key_rid_to_idx(&key, &value, idx, self.comparator)
                .is_ne()
        {
            return Err(BTreeError::NotFound);
        }

        // Check if key has already been deleted
        if leaf.is_idx_tombstoned(idx) {
            return Err(BTreeError::NotFound);
        }

        let leaf_is_root = leaf_page_id == root_page_id;
        let leaf_is_safe = self.is_leaf_delete_safe(&leaf, leaf_is_root);

        // Note that the leaf guard is already popped from the context, everything
        // else we no longer need, either we an optimistic delete with the leaf guard alone,
        // or pessimistic delete will reattempt to get all the necessary latches from scratch
        ctx.release_all();

        if !leaf_is_safe {
            drop(leaf_guard);
            return self.remove_pessimistic(key, value);
        }

        self.delete_from_leaf(&mut leaf, idx);

        Ok(())
    }

    fn remove_pessimistic(&self, key: K, value: Rid) -> Result<(), BTreeError> {
        let mut ctx = BTreeContext::new();
        ctx.header = Some(self.header_guard_mut()?);

        let root_page_id =
            BTreeRootPage::from_data(ctx.header.as_ref().unwrap().data()).root_page_id();
        assert_ne!(root_page_id, INVALID_PAGE_ID);

        ctx.set_root_page_id(root_page_id);
        ctx.write_set.push(self.bpm.write_page(root_page_id)?);
        self.descend_write_path_for_delete(&mut ctx, key)?;

        let mut leaf_guard = ctx.write_set.pop().unwrap();
        let leaf_page_id = leaf_guard.page_id();
        let mut leaf = Self::leaf_page_mut(leaf_guard.data_mut());
        let delete_idx = leaf.find_insert_pos(&key, &value, self.comparator);

        // Verify again that the thing we want to delete is valid
        if delete_idx >= leaf.curr_size()
            || leaf
                .cmp_key_rid_to_idx(&key, &value, delete_idx, self.comparator)
                .is_ne()
            || leaf.is_idx_tombstoned(delete_idx)
        {
            return Err(BTreeError::NotFound);
        }

        let is_safe = leaf.is_delete_safe();
        let physically_deleted = self.delete_from_leaf(&mut leaf, delete_idx);

        if is_safe || !physically_deleted {
            return Ok(());
        }

        // Leaf is also the root page of tree, it could be underweight
        if ctx.write_set.is_empty() {
            // Deallocate page if the tree is completely empty
            if leaf.curr_size() == 0 {
                BTreeRootPageMut::from_data(ctx.header.unwrap().data_mut())
                    .set_root_page_id(INVALID_PAGE_ID);
                drop(leaf_guard);
                self.bpm.delete_page(leaf_page_id)?;
            }

            return Ok(());
        }

        // From this point onwards, we know that we MUST rebalance and that we
        // have at least one parent.
        let mut parent_guard = ctx.write_set.last_mut().unwrap();
        let mut parent = Self::internal_page_mut(parent_guard.data_mut());
        let leaf_idx = parent
            .value_idx(&leaf_page_id)
            .expect("leaf has to belong to parent");
        let left_sibling_idx = leaf_idx.checked_sub(1);
        let right_sibling_idx = leaf_idx + 1;

        // Try borrowing with leaf sibling first
        if let Some(left_sibling_idx) = left_sibling_idx {
            let mut sibling_guard = self.bpm.write_page(*parent.value_at(left_sibling_idx))?;
            let mut sibling_page = Self::leaf_page_mut(sibling_guard.data_mut());

            if Self::try_borrow_from_left_leaf(&mut parent, leaf_idx, &mut leaf, &mut sibling_page)
            {
                return Ok(());
            }
        }

        // Try borrowing with right sibling next
        if right_sibling_idx < parent.curr_size() {
            let mut sibling_guard = self.bpm.write_page(*parent.value_at(right_sibling_idx))?;
            let mut sibling_page = Self::leaf_page_mut(sibling_guard.data_mut());

            if Self::try_borrow_from_right_leaf(
                &mut parent,
                right_sibling_idx,
                &mut leaf,
                &mut sibling_page,
            ) {
                return Ok(());
            }
        }

        // Try merging with leaf sibling
        if let Some(left_sibling_idx) = left_sibling_idx {
            let mut sibling_guard = self.bpm.write_page(*parent.value_at(left_sibling_idx))?;
            let mut sibling_page = Self::leaf_page_mut(sibling_guard.data_mut());
            if Self::can_merge_right_into_leaf(&sibling_page, &leaf) {
                sibling_page.coalesce_right_into_page(&mut leaf);
                parent.remove_at(leaf_idx);
                drop(leaf_guard);
                drop(sibling_guard);
                self.bpm.delete_page(leaf_page_id)?;

                return self.propagate_parent_underflow(ctx);
            }
        }

        // Try merging with right sibling
        if right_sibling_idx < parent.curr_size() {
            let mut sibling_guard = self.bpm.write_page(*parent.value_at(right_sibling_idx))?;
            let sibling_page_id = sibling_guard.page_id();
            let mut sibling_page = Self::leaf_page_mut(sibling_guard.data_mut());

            if Self::can_merge_right_into_leaf(&leaf, &sibling_page) {
                leaf.coalesce_right_into_page(&mut sibling_page);
                parent.remove_at(right_sibling_idx);
                drop(leaf_guard);
                drop(sibling_guard);

                self.bpm.delete_page(sibling_page_id)?;

                return self.propagate_parent_underflow(ctx);
            }
        }

        panic!("delete rebalancing invariant violated: neither sibling can lend or merge")
    }

    fn propagate_parent_underflow(&self, ctx: BTreeContext) -> Result<(), BTreeError> {
        todo!()
    }

    fn redistribute_leaf_pair(
        parent: &mut BTreeInternalPageMut<'_, K>,
        left: &mut BTreeLeafPageMut<'_, K, TOMB_CAP>,
        right: &mut BTreeLeafPageMut<'_, K, TOMB_CAP>,
        right_idx: usize,
    ) {
        assert!(left.live_size() + right.live_size() > left.max_size());

        left.remove_all_tombstones();
        right.remove_all_tombstones();

        // Check who is underweight, and make them have at least min size
        if left.curr_size() < left.min_size() {
            let required_count = left.min_size() - left.curr_size();
            assert!(right.curr_size() >= right.min_size() + required_count);

            for _ in 0..required_count {
                let borrowed_key = *right.key_ref(0);
                let borrowed_value = *right.rid_ref(0);
                left.insert_at(left.curr_size(), &borrowed_key, &borrowed_value);
                right.remove_at(0);
            }
        } else if right.curr_size() < right.min_size() {
            let required_count = right.min_size() - right.curr_size();
            assert!(left.curr_size() >= left.min_size() + required_count);

            // todo: this is definitely not the most efficient way to copy,
            // optimise this later
            for _ in 0..required_count {
                let borrowed_key = *left.key_ref(left.curr_size() - 1);
                let borrowed_value = *left.rid_ref(left.curr_size() - 1);
                right.insert_at(0, &borrowed_key, &borrowed_value);
                left.remove_at(left.curr_size() - 1);
            }
        } else {
            // We should never use this function in this way
            panic!("logic error");
        }

        let right_sep_key = *right.key_ref(0);
        let right_sep_value = *right.rid_ref(0);

        parent.set_index_key_at(right_idx, &right_sep_key, &right_sep_value);
    }

    fn try_borrow_from_left_leaf(
        parent: &mut BTreeInternalPageMut<'_, K>,
        leaf_idx: usize,
        leaf: &mut BTreeLeafPageMut<'_, K, TOMB_CAP>,
        donor: &mut BTreeLeafPageMut<'_, K, TOMB_CAP>,
    ) -> bool {
        if !donor.is_delete_safe() {
            return false;
        }

        let donated_idx = donor.curr_size() - 1;
        let donation_is_tombstoned = donor.is_idx_tombstoned(donated_idx);

        if donation_is_tombstoned && leaf.are_tombstones_full() {
            // We could make space in our tombstone array to accommodate it,
            // but that might mean that we would have to borrow more than one
            // key, so forget it
            return false;
        }

        let borrowed_key = *donor.key_ref(donated_idx);
        let borrowed_value = *donor.rid_ref(donated_idx);
        leaf.insert_at(0, &borrowed_key, &borrowed_value);
        if donation_is_tombstoned {
            leaf.add_tombstone(0);
        }

        donor.remove_at(donated_idx);
        // Update separator keys of the parent as the leaf now has an updated
        // first key that could be inferior to the previous separator
        parent.set_index_key_at(leaf_idx, &borrowed_key, &borrowed_value);

        true
    }

    fn try_borrow_from_right_leaf(
        parent: &mut BTreeInternalPageMut<'_, K>,
        donor_idx: usize,
        leaf: &mut BTreeLeafPageMut<'_, K, TOMB_CAP>,
        donor: &mut BTreeLeafPageMut<'_, K, TOMB_CAP>,
    ) -> bool {
        if !donor.is_delete_safe() {
            return false;
        }

        let donated_idx = 0;
        let donation_is_tombstoned = donor.is_idx_tombstoned(donated_idx);

        if donation_is_tombstoned && leaf.are_tombstones_full() {
            return false;
        }

        let borrowed_key = *donor.key_ref(donated_idx);
        let borrowed_value = *donor.rid_ref(donated_idx);
        let leaf_size = leaf.curr_size();
        leaf.insert_at(leaf_size, &borrowed_key, &borrowed_value);
        if donation_is_tombstoned {
            leaf.add_tombstone(leaf_size);
        }

        donor.remove_at(donated_idx);
        // Update the donor child's parent separator because donor's first entry changed.
        debug_assert!(donor.curr_size() > 0);
        parent.set_index_key_at(donor_idx, donor.key_ref(0), donor.rid_ref(0));

        true
    }

    fn can_merge_right_into_leaf(
        left: &BTreeLeafPageMut<'_, K, TOMB_CAP>,
        right: &BTreeLeafPageMut<'_, K, TOMB_CAP>,
    ) -> bool {
        // Merge means moving all entries from `right` into `left`.
        // Without tombstones, this is safe after borrow fails because both pages are
        // small enough to fit together.
        //
        // With tombstones, two things can prevent a direct merge:
        // - the combined physical entry count may exceed max_size;
        // - the combined tombstone count may exceed TOMB_CAP.
        //
        // We can make room by physically removing tombstoned entries before/during the
        // merge. Removing tombstoned entries reduces both physical size and tombstone
        // count, and does not remove live tuples.

        let total_entries = left.curr_size() + right.curr_size();
        let total_tombstones = left.get_tombstone_count() + right.get_tombstone_count();

        debug_assert!(total_tombstones <= total_entries);

        let total_entries = left.curr_size() + right.curr_size();
        let total_tombstones = left.get_tombstone_count() + right.get_tombstone_count();

        let required_prunes = total_entries
            .saturating_sub(left.max_size())
            .max(total_tombstones.saturating_sub(TOMB_CAP));

        required_prunes <= total_tombstones && total_entries - required_prunes >= left.min_size()
    }

    // Returns true if deletion physically removed any entry from the leaf.
    fn delete_from_leaf(&self, leaf: &mut BTreeLeafPageMut<'_, K, TOMB_CAP>, idx: usize) -> bool {
        if TOMB_CAP == 0 {
            leaf.remove_at(idx);
            return true;
        }

        if !leaf.are_tombstones_full() {
            leaf.add_tombstone(idx);
            return false;
        }

        leaf.evict_oldest_tombstone_and_append(idx);

        true
    }

    fn descend_read_path_for_delete(
        &self,
        ctx: &mut BTreeContext<'a>,
        key: K,
    ) -> Result<(), BTreeError> {
        todo!()
    }

    fn descend_write_path_for_delete(
        &self,
        ctx: &mut BTreeContext<'a>,
        key: K,
    ) -> Result<(), BTreeError> {
        todo!()
    }

    fn is_leaf_delete_safe(&self, leaf: &BTreeLeafPageMut<'_, K, TOMB_CAP>, is_root: bool) -> bool {
        // A leaf is safe for optimistic deletion if deleting from it cannot require
        // borrowing/merging with a sibling.
        // - A root leaf can be underweight, but leaf-only deletion must not physically
        //   remove the final tuple.
        // - If there is a free tombstone slot, deletion only adds a tombstone and
        //   does not reduce physical size.
        // - If tombstones are full, deletion evicts one physical entry, so the leaf
        //   must be above min size.

        if is_root {
            leaf.curr_size() > 1 || TOMB_CAP > 0
        } else {
            !leaf.are_tombstones_full() || leaf.is_delete_safe()
        }
    }

    // Iteration uses panicking semantics for buffer pool failures.
    pub fn iter(&self) -> IndexIterator<'_, K, TOMB_CAP> {
        let mut curr_page_guard = {
            let header_guard = self.header_guard().unwrap();
            let header_page = BTreeRootPage::from_data(header_guard.data());
            let curr_page_id = header_page.root_page_id();

            if curr_page_id == INVALID_PAGE_ID {
                return IndexIterator::new(self.bpm, None);
            }
            self.bpm.read_page(curr_page_id).unwrap()
        };

        while !BTreeNodeHeader::from_data(curr_page_guard.data()).is_leaf() {
            let curr_internal_page = Self::internal_page(curr_page_guard.data());
            assert!(
                curr_internal_page.curr_size() > 0,
                "empty internal page is invalid"
            );

            let child_page_id = curr_internal_page.value_at(0);
            curr_page_guard = self.bpm.read_page(*child_page_id).unwrap();
        }

        IndexIterator::new(self.bpm, Some(curr_page_guard))
    }

    pub fn lower_bound(&self, key: &K) -> IndexIterator<'_, K, TOMB_CAP> {
        let mut curr_page_guard = {
            let header_guard = self.header_guard().unwrap();
            let header_page = BTreeRootPage::from_data(header_guard.data());
            let curr_page_id = header_page.root_page_id();

            if curr_page_id == INVALID_PAGE_ID {
                return IndexIterator::new(self.bpm, None);
            }
            self.bpm.read_page(curr_page_id).unwrap()
        };

        while !BTreeNodeHeader::from_data(curr_page_guard.data()).is_leaf() {
            let curr_internal_page = Self::internal_page(curr_page_guard.data());
            assert!(
                curr_internal_page.curr_size() > 0,
                "empty internal page is invalid"
            );

            let child_page_id = curr_internal_page
                .value_at(curr_internal_page.find_child_idx(key, self.comparator));
            curr_page_guard = self.bpm.read_page(*child_page_id).unwrap();
        }

        let idx = {
            let leaf = Self::leaf_page(curr_page_guard.data());
            leaf.find_pos(key, self.comparator)
        };

        IndexIterator::new_at(self.bpm, Some(curr_page_guard), idx)
    }
}

#[cfg(test)]
mod tests {
    use std::{cmp::Ordering, path::PathBuf};

    use tempfile::NamedTempFile;

    use super::*;
    use crate::storage::disk::disk_manager::DiskManager;

    struct U64Comparator;

    impl KeyComparator<u64> for U64Comparator {
        fn compare(&self, a: &u64, b: &u64) -> Ordering {
            a.cmp(b)
        }
    }

    fn setup_bpm(pool_size: usize) -> BufferPoolManager {
        let file = NamedTempFile::new().unwrap();
        let disk_manager = DiskManager::new(PathBuf::from(file.path())).unwrap();
        BufferPoolManager::new(pool_size, disk_manager)
    }

    fn rid_for_key(key: u64) -> Rid {
        Rid::new(
            (key / (u16::MAX as u64 + 1)) as PageId,
            (key & 0xffff) as usize,
        )
    }

    fn root_page_id(bpm: &BufferPoolManager, header_page_id: PageId) -> PageId {
        let header_guard = bpm.read_page(header_page_id).unwrap();
        BTreeRootPage::from_data(header_guard.data()).root_page_id()
    }

    fn find_insert_safe_leaf_key<const TOMB_CAP: usize>(
        bpm: &BufferPoolManager,
        root_page_id: PageId,
    ) -> Option<u64> {
        let mut current_page_id = root_page_id;

        loop {
            let current_guard = bpm.read_page(current_page_id).unwrap();
            if BTreeNodeHeader::from_data(current_guard.data()).is_leaf() {
                break;
            }

            let internal_page = BTreeInternalPage::<u64>::from_data(current_guard.data());
            current_page_id = *internal_page.value_at(0);
        }

        loop {
            let leaf_guard = bpm.read_page(current_page_id).unwrap();
            let leaf = BTreeLeafPage::<u64, TOMB_CAP>::from_data(leaf_guard.data());

            if leaf.curr_size() > 0 && leaf.curr_size() + 1 < leaf.max_size() {
                return Some(*leaf.key_at(0) + 1);
            }

            let next_page_id = leaf.get_next_page_id();
            if next_page_id == INVALID_PAGE_ID {
                return None;
            }

            current_page_id = next_page_id;
        }
    }

    #[test]
    fn basic_insert_test() {
        const TOMB_CAP: usize = 3;

        let bpm = setup_bpm(50);
        let header_page_id = bpm.new_page();
        let comparator = U64Comparator;
        let tree = BTree::<u64, _, TOMB_CAP>::new(&bpm, header_page_id, &comparator);

        let key = 42;
        let rid = rid_for_key(key);
        tree.insert(key, rid).unwrap();

        let root_page_id = root_page_id(&bpm, header_page_id);
        let root_guard = bpm.read_page(root_page_id).unwrap();
        assert!(BTreeNodeHeader::from_data(root_guard.data()).is_leaf());

        let root_as_leaf = BTreeLeafPage::<u64, TOMB_CAP>::from_data(root_guard.data());
        assert_eq!(root_as_leaf.curr_size(), 1);
        assert_eq!(*root_as_leaf.key_at(0), key);
        assert_eq!(tree.get_values(&key).unwrap(), vec![rid]);
    }

    #[test]
    fn insert_test_1_no_iterator() {
        const TOMB_CAP: usize = 3;

        let bpm = setup_bpm(50);
        let header_page_id = bpm.new_page();
        let comparator = U64Comparator;
        let tree = BTree::<u64, _, TOMB_CAP>::new(&bpm, header_page_id, &comparator);

        let keys = [1, 2, 3, 4, 5];
        for key in keys {
            tree.insert(key, rid_for_key(key)).unwrap();
        }

        for key in keys {
            let rids = tree.get_values(&key).unwrap();
            assert_eq!(rids, vec![rid_for_key(key)]);
        }
    }

    #[test]
    fn get_values_returns_empty_for_missing_key() {
        const TOMB_CAP: usize = 3;

        let bpm = setup_bpm(50);
        let header_page_id = bpm.new_page();
        let comparator = U64Comparator;
        let tree = BTree::<u64, _, TOMB_CAP>::new(&bpm, header_page_id, &comparator);

        for key in [1, 3, 5, 7, 9] {
            tree.insert(key, rid_for_key(key)).unwrap();
        }

        assert_eq!(tree.get_values(&0).unwrap(), Vec::<Rid>::new());
        assert_eq!(tree.get_values(&4).unwrap(), Vec::<Rid>::new());
        assert_eq!(tree.get_values(&10).unwrap(), Vec::<Rid>::new());
    }

    #[test]
    fn get_values_returns_duplicate_key_rids_across_leaf_boundaries() {
        const TOMB_CAP: usize = 3;

        let bpm = setup_bpm(50);
        let header_page_id = bpm.new_page();
        let comparator = U64Comparator;
        let tree = BTree::<u64, _, TOMB_CAP>::new(&bpm, header_page_id, &comparator);

        let key = 42;
        let duplicate_count = BTreeLeafPageMut::<u64, TOMB_CAP>::MAX_SIZE + 20;
        let expected: Vec<_> = (0..duplicate_count)
            .map(|idx| Rid::new((idx / 128) as PageId, idx % 128))
            .collect();

        for rid in expected.iter().rev() {
            tree.insert(key, *rid).unwrap();
        }

        let root_page_id = root_page_id(&bpm, header_page_id);
        let root_guard = bpm.read_page(root_page_id).unwrap();
        assert!(!BTreeNodeHeader::from_data(root_guard.data()).is_leaf());

        assert_eq!(tree.get_values(&key).unwrap(), expected);
    }

    #[test]
    fn out_of_order_insert_stress_creates_multiple_levels() {
        const TOMB_CAP: usize = 3;
        const NUM_KEYS: u64 = 220_000;
        const MULTIPLIER: u64 = 37_211;

        let bpm = setup_bpm(1_000);
        let header_page_id = bpm.new_page();
        let comparator = U64Comparator;
        let tree = BTree::<u64, _, TOMB_CAP>::new(&bpm, header_page_id, &comparator);

        // this inserts permutation of the 0..NUM_KEYS because
        // multiplication by a unit modulo n is a permutation of Z/nZ.
        for i in 0..NUM_KEYS {
            let key = (i * MULTIPLIER) % NUM_KEYS;
            tree.insert(key, rid_for_key(key)).unwrap();
        }

        let root_page_id = root_page_id(&bpm, header_page_id);
        let root_guard = bpm.read_page(root_page_id).unwrap();
        let root = BTreeInternalPage::<u64>::from_data(root_guard.data());

        for key in 0..NUM_KEYS {
            assert_eq!(tree.get_values(&key).unwrap(), vec![rid_for_key(key)]);
        }
    }

    #[test]
    fn iter_scans_out_of_order_in_key_order() {
        const TOMB_CAP: usize = 3;
        const NUM_KEYS: u64 = 1_000;
        const MULTIPLIER: u64 = 37;

        let bpm = setup_bpm(100);
        let header_page_id = bpm.new_page();
        let comparator = U64Comparator;
        let tree = BTree::<u64, _, TOMB_CAP>::new(&bpm, header_page_id, &comparator);

        for i in 0..NUM_KEYS {
            let key = (i * MULTIPLIER) % NUM_KEYS;
            tree.insert(key, rid_for_key(key)).unwrap();
        }

        let scanned: Vec<_> = tree.iter().collect();
        let expected: Vec<_> = (0..NUM_KEYS).map(|key| (key, rid_for_key(key))).collect();

        assert_eq!(scanned, expected);
    }

    #[test]
    fn lower_bound_starts_at_first_key_not_less_than_search_key() {
        const TOMB_CAP: usize = 3;
        const NUM_KEYS: u64 = 1_000;
        const MULTIPLIER: u64 = 37;

        let bpm = setup_bpm(100);
        let header_page_id = bpm.new_page();
        let comparator = U64Comparator;
        let tree = BTree::<u64, _, TOMB_CAP>::new(&bpm, header_page_id, &comparator);

        for i in 0..NUM_KEYS {
            let key = ((i * MULTIPLIER) % NUM_KEYS) * 2 + 1;
            tree.insert(key, rid_for_key(key)).unwrap();
        }

        let scanned: Vec<_> = tree.lower_bound(&500).take(5).collect();
        let expected: Vec<_> = [501, 503, 505, 507, 509]
            .into_iter()
            .map(|key| (key, rid_for_key(key)))
            .collect();
        assert_eq!(scanned, expected);

        assert_eq!(tree.lower_bound(&0).next(), Some((1, rid_for_key(1))));
        assert_eq!(tree.lower_bound(&(NUM_KEYS * 2 + 1)).next(), None);
    }

    #[test]
    fn lower_bound_finds_first_duplicate_across_leaf_boundaries() {
        const TOMB_CAP: usize = 3;

        let bpm = setup_bpm(50);
        let header_page_id = bpm.new_page();
        let comparator = U64Comparator;
        let tree = BTree::<u64, _, TOMB_CAP>::new(&bpm, header_page_id, &comparator);

        let key = 42;
        let duplicate_count = BTreeLeafPageMut::<u64, TOMB_CAP>::MAX_SIZE + 20;
        let expected_rids: Vec<_> = (0..duplicate_count)
            .map(|idx| Rid::new((idx / 128) as PageId, idx % 128))
            .collect();

        for rid in expected_rids.iter().rev() {
            tree.insert(key, *rid).unwrap();
        }

        let scanned: Vec<_> = tree.lower_bound(&key).take(duplicate_count).collect();
        let expected: Vec<_> = expected_rids.into_iter().map(|rid| (key, rid)).collect();

        assert_eq!(scanned, expected);
    }

    #[test]
    fn optimistic_insert_test() {
        const TOMB_CAP: usize = 3;

        let bpm = setup_bpm(100);
        let header_page_id = bpm.new_page();
        let comparator = U64Comparator;
        let tree = BTree::<u64, _, TOMB_CAP>::new(&bpm, header_page_id, &comparator);

        let num_keys = BTreeLeafPageMut::<u64, TOMB_CAP>::MAX_SIZE * 2;
        for i in 0..num_keys {
            let key = 2 * i as u64;
            tree.insert(key, rid_for_key(key)).unwrap();
        }

        let root_page_id = root_page_id(&bpm, header_page_id);
        let to_insert = find_insert_safe_leaf_key::<TOMB_CAP>(&bpm, root_page_id)
            .expect("expected an insert-safe leaf");
        assert!(tree.get_values(&to_insert).unwrap().is_empty());

        let base_reads = bpm.read_count();
        let base_writes = bpm.write_count();

        let rid = rid_for_key(to_insert);
        tree.insert(to_insert, rid).unwrap();

        // Inserting optimistically means that we only do reads all the way
        // down to the leaf, after which we do a single write.
        assert!(bpm.read_count() - base_reads > 0);
        assert_eq!(bpm.write_count() - base_writes, 1);
        assert_eq!(tree.get_values(&to_insert).unwrap(), vec![rid]);
    }
}
