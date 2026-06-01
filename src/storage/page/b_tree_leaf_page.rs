use std::marker::PhantomData;

use bytemuck::Pod;

use crate::{
    buffer::page::{INVALID_PAGE_ID, PageBytes},
    common::{
        alignment::{align_up, max_usize},
        types::PageId,
    },
    storage::{
        disk::config::DEFAULT_PAGE_SIZE,
        index::comparator::KeyComparator,
        page::b_tree_node_header::{BTreeNodeHeader, PAGE_TYPE_LEAF},
        rid::Rid,
    },
};

struct BTreeLeafPageView<'a, K, const TOMB_CAP: usize> {
    data: &'a PageBytes,
    _marker: PhantomData<K>,
}

pub struct BTreeLeafPage<'a, K, const TOMB_CAP: usize> {
    view: BTreeLeafPageView<'a, K, TOMB_CAP>,
}

pub struct BTreeLeafPageMut<'a, K, const TOMB_CAP: usize> {
    data: &'a mut PageBytes,
    _marker: PhantomData<K>,
}

#[repr(transparent)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TombstoneIndex(pub u16);

impl From<TombstoneIndex> for usize {
    fn from(value: TombstoneIndex) -> Self {
        value.0 as usize
    }
}

impl TombstoneIndex {
    pub fn incr(&mut self) {
        self.0 += 1;
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BTreeLeafHeader {
    common: BTreeNodeHeader,
    next_page_id: u32,
    num_tombstones: u16,
    _reserved: u16,
    // 16 bytes header
}

impl<'a, K: Pod, const TOMB_CAP: usize> BTreeLeafPageView<'a, K, TOMB_CAP> {
    const HEADER_SIZE: usize = size_of::<BTreeLeafHeader>();
    const TOMBSTONES_OFFSET: usize = Self::HEADER_SIZE;
    const ENTRY_ALIGN: usize = max_usize(align_of::<K>(), align_of::<Rid>());
    const ENTRIES_OFFSET: usize = align_up(
        Self::TOMBSTONES_OFFSET + TOMB_CAP * size_of::<TombstoneIndex>(),
        Self::ENTRY_ALIGN,
    );
    const ENTRY_RID_OFFSET: usize = align_up(size_of::<K>(), align_of::<Rid>());
    const ENTRY_SIZE: usize =
        align_up(Self::ENTRY_RID_OFFSET + size_of::<Rid>(), Self::ENTRY_ALIGN);

    fn from_data(data: &'a PageBytes) -> Self {
        Self {
            data,
            _marker: PhantomData,
        }
    }

    fn header(&self) -> &BTreeLeafHeader {
        let header_bytes = &self.data[..size_of::<BTreeLeafHeader>()];
        bytemuck::from_bytes(header_bytes)
    }

    fn get_tombstone_count(&self) -> usize {
        self.header().num_tombstones as usize
    }

    fn get_next_page_id(&self) -> PageId {
        self.header().next_page_id
    }

    fn max_size(&self) -> usize {
        self.header().common.max_size as usize
    }

    fn curr_size(&self) -> usize {
        self.header().common.current_size as usize
    }

    fn tombstones(&self) -> &[TombstoneIndex] {
        let start = Self::TOMBSTONES_OFFSET;
        let end = start + TOMB_CAP * size_of::<TombstoneIndex>();
        bytemuck::cast_slice(&self.data[start..end])
    }

    fn entry_offset(idx: usize) -> usize {
        Self::ENTRIES_OFFSET + idx * Self::ENTRY_SIZE
    }

    fn key_ref(&self, idx: usize) -> &K {
        let start = Self::entry_offset(idx);
        let end = start + size_of::<K>();
        bytemuck::from_bytes(&self.data[start..end])
    }

    fn rid_ref(&self, idx: usize) -> &Rid {
        let start = Self::entry_offset(idx) + Self::ENTRY_RID_OFFSET;
        let end = start + size_of::<Rid>();
        bytemuck::from_bytes(&self.data[start..end])
    }

    fn key_at(&self, idx: usize) -> &K {
        assert!(idx < self.curr_size());
        self.key_ref(idx)
    }

    fn value_at(&self, idx: usize) -> &Rid {
        assert!(idx < self.curr_size());
        self.rid_ref(idx)
    }

    fn lower_bound_by<F>(&self, compare_entry: F) -> usize
    where
        F: Fn(usize) -> std::cmp::Ordering,
    {
        let mut left = 0;
        let mut right = self.header().common.current_size as usize;

        while left < right {
            let mid = left + ((right - left) / 2);

            match compare_entry(mid) {
                std::cmp::Ordering::Less => {
                    left = mid + 1;
                }
                _ => {
                    right = mid;
                }
            }
        }

        left
    }

    fn find_pos<C>(&self, key: &K, c: &C) -> usize
    where
        C: KeyComparator<K>,
    {
        self.lower_bound_by(|idx| c.compare(self.key_ref(idx), key))
    }

    fn find_insert_pos<C>(&self, key: &K, rid: &Rid, c: &C) -> usize
    where
        C: KeyComparator<K>,
    {
        self.lower_bound_by(|idx| self.cmp_key_rid_to_idx(key, rid, idx, c))
    }

    fn cmp_key_rid_to_idx<C>(&self, key: &K, rid: &Rid, idx: usize, c: &C) -> std::cmp::Ordering
    where
        C: KeyComparator<K>,
    {
        c.compare(self.key_ref(idx), key)
            .then_with(|| compare_rid(self.rid_ref(idx), rid))
    }

    fn is_idx_tombstoned(&self, idx: usize) -> bool {
        let tombstones = self.tombstones();

        for i in 0..self.get_tombstone_count() {
            if usize::from(tombstones[i]) == idx {
                return true;
            }
        }

        false
    }

    fn get_tombstoned_keys(&self) -> Vec<K> {
        self.tombstones()[..self.get_tombstone_count()]
            .iter()
            .map(|idx| *self.key_ref(usize::from(*idx)))
            .collect()
    }

    fn is_insert_safe(&self) -> bool {
        // Since we split leaf pages if their size == maxSize after insertion, we need
        // to ensure that even after adding one entry, the size is still < maxSize
        self.curr_size() + 1 < self.max_size()
    }
}

impl<'a, K: Pod + Copy, const TOMB_CAP: usize> BTreeLeafPage<'a, K, TOMB_CAP> {
    pub fn from_data(data: &'a PageBytes) -> Self {
        Self {
            view: BTreeLeafPageView::from_data(data),
        }
    }

    fn header(&self) -> &BTreeLeafHeader {
        self.view.header()
    }

    pub fn get_tombstone_count(&self) -> usize {
        self.view.get_tombstone_count()
    }

    pub fn get_next_page_id(&self) -> PageId {
        self.view.get_next_page_id()
    }

    pub fn max_size(&self) -> usize {
        self.view.max_size()
    }

    pub fn curr_size(&self) -> usize {
        self.view.curr_size()
    }

    pub fn get_tombstoned_keys(&self) -> Vec<K> {
        self.view.get_tombstoned_keys()
    }

    pub fn key_at(&self, idx: usize) -> &K {
        self.view.key_at(idx)
    }

    pub fn value_at(&self, idx: usize) -> &Rid {
        self.view.value_at(idx)
    }

    pub fn find_pos<C>(&self, key: &K, c: &C) -> usize
    where
        C: KeyComparator<K>,
    {
        self.view.find_pos(key, c)
    }

    pub fn find_insert_pos<C>(&self, key: &K, rid: &Rid, c: &C) -> usize
    where
        C: KeyComparator<K>,
    {
        self.view.find_insert_pos(key, rid, c)
    }

    pub fn is_idx_tombstoned(&self, idx: usize) -> bool {
        self.view.is_idx_tombstoned(idx)
    }

    pub fn cmp_key_rid_to_idx<C>(&self, key: &K, rid: &Rid, idx: usize, c: &C) -> std::cmp::Ordering
    where
        C: KeyComparator<K>,
    {
        self.view.cmp_key_rid_to_idx(key, rid, idx, c)
    }

    pub fn is_insert_safe(&self) -> bool {
        self.view.is_insert_safe()
    }
}

impl<'a, K: Pod, const TOMB_CAP: usize> BTreeLeafPageMut<'a, K, TOMB_CAP> {
    pub const MAX_SIZE: usize = leaf_page_max_size::<K, TOMB_CAP>();

    pub fn from_data(data: &'a mut PageBytes) -> Self {
        Self {
            data,
            _marker: PhantomData,
        }
    }

    pub fn init(data: &'a mut PageBytes) -> Self {
        data.fill(0);

        let mut page = Self::from_data(data);
        let header = page.header_mut();
        header.common.init(PAGE_TYPE_LEAF, Self::MAX_SIZE);
        header.next_page_id = INVALID_PAGE_ID as u32;
        header.num_tombstones = 0;
        header._reserved = 0;

        page
    }

    fn view(&self) -> BTreeLeafPageView<'_, K, TOMB_CAP> {
        BTreeLeafPageView::from_data(&*self.data)
    }

    fn header(&self) -> &BTreeLeafHeader {
        let header_bytes = &self.data[..size_of::<BTreeLeafHeader>()];
        bytemuck::from_bytes(header_bytes)
    }

    fn header_mut(&mut self) -> &mut BTreeLeafHeader {
        let header_bytes = &mut self.data[..size_of::<BTreeLeafHeader>()];
        bytemuck::from_bytes_mut(header_bytes)
    }

    fn tombstones(&self) -> &[TombstoneIndex] {
        let start = BTreeLeafPageView::<K, TOMB_CAP>::TOMBSTONES_OFFSET;
        let end = start + TOMB_CAP * size_of::<TombstoneIndex>();
        bytemuck::cast_slice(&self.data[start..end])
    }

    fn tombstones_mut(&mut self) -> &mut [TombstoneIndex] {
        let start = BTreeLeafPageView::<K, TOMB_CAP>::TOMBSTONES_OFFSET;
        let end = start + TOMB_CAP * size_of::<TombstoneIndex>();
        bytemuck::cast_slice_mut(&mut self.data[start..end])
    }

    pub fn max_size(&self) -> usize {
        self.view().max_size()
    }

    fn curr_size(&self) -> usize {
        self.view().curr_size()
    }

    fn num_tombstones(&self) -> usize {
        self.view().get_tombstone_count()
    }

    fn entry_offset(idx: usize) -> usize {
        BTreeLeafPageView::<K, TOMB_CAP>::entry_offset(idx)
    }

    fn key_ref(&self, idx: usize) -> &K {
        let start = Self::entry_offset(idx);
        let end = start + size_of::<K>();
        bytemuck::from_bytes(&self.data[start..end])
    }

    fn rid_ref(&self, idx: usize) -> &Rid {
        let start = Self::entry_offset(idx) + BTreeLeafPageView::<K, TOMB_CAP>::ENTRY_RID_OFFSET;
        let end = start + size_of::<Rid>();
        bytemuck::from_bytes(&self.data[start..end])
    }

    pub fn set_next_page_id(&mut self, page_id: PageId) {
        self.header_mut().next_page_id = page_id;
    }

    pub fn set_size(&mut self, size: usize) {
        self.header_mut().common.current_size = size as u16;
    }

    pub fn set_num_tombstones(&mut self, size: usize) {
        self.header_mut().num_tombstones = size as u16;
    }

    pub fn find_pos<C>(&self, key: &K, c: &C) -> usize
    where
        C: KeyComparator<K>,
    {
        self.view().find_pos(key, c)
    }

    pub fn find_insert_pos<C>(&self, key: &K, rid: &Rid, c: &C) -> usize
    where
        C: KeyComparator<K>,
    {
        self.view().find_insert_pos(key, rid, c)
    }

    // Caller must ensure that the page is not already full, if this condition
    // is not respected, this function will panic
    pub fn insert_at(&mut self, idx: usize, key: &K, rid: &Rid) {
        let tombstone_count = self.header().num_tombstones as usize;
        let size = self.header().common.current_size as usize;

        assert!(idx <= size && size < self.max_size());

        for tombstone in self.tombstones_mut()[..tombstone_count].iter_mut() {
            if usize::from(*tombstone) >= idx {
                tombstone.incr();
            }
        }

        for i in (idx..size).rev() {
            self.copy_entry(i, i + 1);
        }

        self.write_entry(idx, key, rid);
        self.header_mut().common.current_size += 1;
    }

    fn copy_entry(&mut self, src_idx: usize, dst_idx: usize) {
        let src_start = Self::entry_offset(src_idx);
        let dst_start = Self::entry_offset(dst_idx);

        self.data.copy_within(
            src_start..src_start + BTreeLeafPageView::<K, TOMB_CAP>::ENTRY_SIZE,
            dst_start,
        );
    }

    fn write_entry(&mut self, idx: usize, key: &K, rid: &Rid) {
        assert!(idx < self.max_size(), "out of bounds");

        let key_start = Self::entry_offset(idx);
        let key_end = key_start + size_of::<K>();
        self.data[key_start..key_end].copy_from_slice(bytemuck::bytes_of(key));

        let rid_start = key_start + BTreeLeafPageView::<K, TOMB_CAP>::ENTRY_RID_OFFSET;
        let rid_end = rid_start + size_of::<Rid>();
        self.data[rid_start..rid_end].copy_from_slice(bytemuck::bytes_of(rid));
    }

    // During insertions, if a leaf page is full, we split its entries into a new
    // sibling page, this function helps move entries to the sibling and resizes
    // the current page. This function assumes that the recipient is a fresh leaf
    // page.
    pub fn move_split_entries_to(&mut self, recipient: &mut Self, start_idx: usize) {
        let size = self.curr_size();
        assert!(start_idx < size, "invalid split index");

        let mut recipient_insert_idx = recipient.curr_size();

        // shift entries with index >= start_idx to the other page
        for i in start_idx..size {
            if self.is_idx_tombstoned(i) {
                continue;
            }

            recipient.write_entry(recipient_insert_idx, self.key_ref(i), self.rid_ref(i));
            recipient_insert_idx += 1;
        }
        recipient.set_size(recipient_insert_idx);
        recipient.set_num_tombstones(0);

        // compact local tombstones and adjust num_tombstones
        let mut remaining_tombstone_idx = 0;
        let num_tombstones = self.num_tombstones();
        let tombstones = self.tombstones_mut();
        for i in 0..num_tombstones {
            if usize::from(tombstones[i]) < start_idx {
                tombstones[remaining_tombstone_idx] = tombstones[i];
                remaining_tombstone_idx += 1;
            }
        }
        self.set_num_tombstones(remaining_tombstone_idx);
        self.set_size(start_idx);
    }

    pub fn is_idx_tombstoned(&self, idx: usize) -> bool {
        self.view().is_idx_tombstoned(idx)
    }
}

fn compare_rid(a: &Rid, b: &Rid) -> std::cmp::Ordering {
    a.page_id
        .cmp(&b.page_id)
        .then_with(|| a.slot_id.cmp(&b.slot_id))
}

const fn leaf_page_max_size<K, const TOMB_CAP: usize>() -> usize {
    let mut n = 0;

    loop {
        let next = n + 1;

        let tombstones_end = size_of::<BTreeLeafHeader>() + TOMB_CAP * size_of::<TombstoneIndex>();

        let entry_align = max_usize(align_of::<K>(), align_of::<Rid>());
        let entries_offset = align_up(tombstones_end, entry_align);
        let entry_rid_offset = align_up(size_of::<K>(), align_of::<Rid>());
        let entry_size = align_up(entry_rid_offset + size_of::<Rid>(), entry_align);
        let entries_end = entries_offset + next * entry_size;

        if entries_end > DEFAULT_PAGE_SIZE {
            return n;
        }

        n = next;
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use crate::{
        buffer::page::{INVALID_PAGE_ID, PageData},
        storage::disk::config::DEFAULT_PAGE_SIZE,
    };

    use super::*;

    struct U64Comparator;

    impl KeyComparator<u64> for U64Comparator {
        fn compare(&self, a: &u64, b: &u64) -> Ordering {
            a.cmp(b)
        }
    }

    #[test]
    fn init_sets_leaf_header_defaults() {
        let mut data = PageData([0xff; DEFAULT_PAGE_SIZE]);

        {
            let _leaf = BTreeLeafPageMut::<u64, 8>::init(&mut data.0);
        }

        let leaf = BTreeLeafPage::<u64, 8>::from_data(&data.0);
        assert!(leaf.header().common.is_leaf());
        assert_eq!(leaf.max_size(), BTreeLeafPageMut::<u64, 8>::MAX_SIZE);
        assert_eq!(leaf.get_next_page_id(), INVALID_PAGE_ID);
        assert_eq!(leaf.get_tombstone_count(), 0);
    }

    #[test]
    fn full_page_entries_round_trip() {
        let mut data = PageData([0; DEFAULT_PAGE_SIZE]);
        let max_size = BTreeLeafPageMut::<u32, 8>::MAX_SIZE;

        {
            let mut leaf = BTreeLeafPageMut::<u32, 8>::init(&mut data.0);
            leaf.header_mut().common.current_size = max_size as u16;

            for idx in 0..max_size {
                leaf.write_entry(idx, &(idx as u32 + 100), &Rid::new((idx + 1) as u32, idx));
            }
        }

        let leaf = BTreeLeafPage::<u32, 8>::from_data(&data.0);

        for idx in 0..max_size {
            assert_eq!(*leaf.key_at(idx), idx as u32 + 100);
            assert_eq!(*leaf.value_at(idx), Rid::new((idx + 1) as u32, idx));
        }
    }

    #[test]
    fn insert_at_shifts_entries_right() {
        let mut data = PageData([0; DEFAULT_PAGE_SIZE]);

        {
            let mut leaf = BTreeLeafPageMut::<u64, 8>::init(&mut data.0);
            leaf.header_mut().common.current_size = 3;
            leaf.write_entry(0, &10, &Rid::new(1, 10));
            leaf.write_entry(1, &30, &Rid::new(1, 30));
            leaf.write_entry(2, &40, &Rid::new(1, 40));

            leaf.insert_at(1, &20, &Rid::new(1, 20));
        }

        let leaf = BTreeLeafPage::<u64, 8>::from_data(&data.0);
        assert_eq!(leaf.curr_size(), 4);
        assert_eq!(*leaf.key_at(0), 10);
        assert_eq!(*leaf.value_at(0), Rid::new(1, 10));
        assert_eq!(*leaf.key_at(1), 20);
        assert_eq!(*leaf.value_at(1), Rid::new(1, 20));
        assert_eq!(*leaf.key_at(2), 30);
        assert_eq!(*leaf.value_at(2), Rid::new(1, 30));
        assert_eq!(*leaf.key_at(3), 40);
        assert_eq!(*leaf.value_at(3), Rid::new(1, 40));
    }

    #[test]
    fn insert_at_allows_empty_and_end_insert() {
        let mut data = PageData([0; DEFAULT_PAGE_SIZE]);

        {
            let mut leaf = BTreeLeafPageMut::<u64, 8>::init(&mut data.0);
            leaf.insert_at(0, &10, &Rid::new(1, 10));
            leaf.insert_at(1, &20, &Rid::new(1, 20));
        }

        let leaf = BTreeLeafPage::<u64, 8>::from_data(&data.0);
        assert_eq!(leaf.curr_size(), 2);
        assert_eq!(*leaf.key_at(0), 10);
        assert_eq!(*leaf.value_at(0), Rid::new(1, 10));
        assert_eq!(*leaf.key_at(1), 20);
        assert_eq!(*leaf.value_at(1), Rid::new(1, 20));
    }

    #[test]
    fn insert_at_panics_when_page_is_full() {
        let mut data = PageData([0; DEFAULT_PAGE_SIZE]);
        let max_size = BTreeLeafPageMut::<u64, 8>::MAX_SIZE;
        let mut leaf = BTreeLeafPageMut::<u64, 8>::init(&mut data.0);

        for idx in 0..max_size {
            leaf.insert_at(idx, &(idx as u64), &Rid::new(1, idx));
        }

        assert_eq!(leaf.curr_size(), max_size);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            leaf.insert_at(max_size, &999, &Rid::new(1, 999));
        }));

        assert!(result.is_err());
    }

    #[test]
    fn insert_at_shifts_tombstone_indexes() {
        let mut data = PageData([0; DEFAULT_PAGE_SIZE]);
        let mut leaf = BTreeLeafPageMut::<u64, 8>::init(&mut data.0);
        leaf.header_mut().common.current_size = 3;
        leaf.header_mut().num_tombstones = 3;
        leaf.write_entry(0, &10, &Rid::new(1, 10));
        leaf.write_entry(1, &30, &Rid::new(1, 30));
        leaf.write_entry(2, &40, &Rid::new(1, 40));
        leaf.tombstones_mut()[..3].copy_from_slice(&[
            TombstoneIndex(0),
            TombstoneIndex(1),
            TombstoneIndex(2),
        ]);

        leaf.insert_at(1, &20, &Rid::new(1, 20));

        assert_eq!(leaf.tombstones_mut()[0].0, 0);
        assert_eq!(leaf.tombstones_mut()[1].0, 2);
        assert_eq!(leaf.tombstones_mut()[2].0, 3);
    }

    #[test]
    fn move_split_entries_to_moves_entries_and_shrinks_source() {
        let mut source_data = PageData([0; DEFAULT_PAGE_SIZE]);
        let mut recipient_data = PageData([0; DEFAULT_PAGE_SIZE]);
        let mut source = BTreeLeafPageMut::<u64, 8>::init(&mut source_data.0);
        let mut recipient = BTreeLeafPageMut::<u64, 8>::init(&mut recipient_data.0);

        source.set_size(4);
        source.write_entry(0, &10, &Rid::new(1, 10));
        source.write_entry(1, &20, &Rid::new(1, 20));
        source.write_entry(2, &30, &Rid::new(1, 30));
        source.write_entry(3, &40, &Rid::new(1, 40));

        source.move_split_entries_to(&mut recipient, 2);

        assert_eq!(source.curr_size(), 2);
        assert_eq!(recipient.curr_size(), 2);
        assert_eq!(*source.key_ref(0), 10);
        assert_eq!(*source.rid_ref(0), Rid::new(1, 10));
        assert_eq!(*source.key_ref(1), 20);
        assert_eq!(*source.rid_ref(1), Rid::new(1, 20));
        assert_eq!(*recipient.key_ref(0), 30);
        assert_eq!(*recipient.rid_ref(0), Rid::new(1, 30));
        assert_eq!(*recipient.key_ref(1), 40);
        assert_eq!(*recipient.rid_ref(1), Rid::new(1, 40));
    }

    #[test]
    fn move_split_entries_to_preserves_left_tombstones_and_skips_moved_tombstones() {
        let mut source_data = PageData([0; DEFAULT_PAGE_SIZE]);
        let mut recipient_data = PageData([0; DEFAULT_PAGE_SIZE]);
        let mut source = BTreeLeafPageMut::<u64, 8>::init(&mut source_data.0);
        let mut recipient = BTreeLeafPageMut::<u64, 8>::init(&mut recipient_data.0);

        source.set_size(5);
        for idx in 0..5 {
            let key = ((idx + 1) * 10) as u64;
            source.write_entry(idx, &key, &Rid::new(1, key as usize));
        }
        source.set_num_tombstones(2);
        source.tombstones_mut()[..2].copy_from_slice(&[TombstoneIndex(1), TombstoneIndex(3)]);

        source.move_split_entries_to(&mut recipient, 2);

        assert_eq!(source.curr_size(), 2);
        assert_eq!(source.num_tombstones(), 1);
        assert_eq!(source.tombstones()[0].0, 1);
        assert_eq!(recipient.curr_size(), 2);
        assert_eq!(recipient.num_tombstones(), 0);
        assert_eq!(*recipient.key_ref(0), 30);
        assert_eq!(*recipient.rid_ref(0), Rid::new(1, 30));
        assert_eq!(*recipient.key_ref(1), 50);
        assert_eq!(*recipient.rid_ref(1), Rid::new(1, 50));
    }

    #[test]
    fn find_pos_returns_first_matching_logical_key() {
        let mut data = PageData([0; DEFAULT_PAGE_SIZE]);
        let mut leaf = BTreeLeafPageMut::<u64, 8>::init(&mut data.0);
        leaf.header_mut().common.current_size = 5;
        leaf.write_entry(0, &10, &Rid::new(1, 1));
        leaf.write_entry(1, &20, &Rid::new(1, 1));
        leaf.write_entry(2, &20, &Rid::new(1, 2));
        leaf.write_entry(3, &30, &Rid::new(1, 1));
        leaf.write_entry(4, &40, &Rid::new(1, 1));

        assert_eq!(leaf.find_pos(&20, &U64Comparator), 1);
        assert_eq!(leaf.find_pos(&25, &U64Comparator), 3);
        assert_eq!(leaf.find_pos(&30, &U64Comparator), 3);
    }

    #[test]
    fn find_insert_pos_uses_rid_as_tiebreaker() {
        let mut data = PageData([0; DEFAULT_PAGE_SIZE]);
        let mut leaf = BTreeLeafPageMut::<u64, 8>::init(&mut data.0);
        leaf.header_mut().common.current_size = 4;
        leaf.write_entry(0, &10, &Rid::new(1, 1));
        leaf.write_entry(1, &20, &Rid::new(1, 1));
        leaf.write_entry(2, &20, &Rid::new(1, 3));
        leaf.write_entry(3, &30, &Rid::new(1, 1));

        assert_eq!(
            leaf.find_insert_pos(&20, &Rid::new(1, 0), &U64Comparator),
            1
        );
        assert_eq!(
            leaf.find_insert_pos(&20, &Rid::new(1, 2), &U64Comparator),
            2
        );
        assert_eq!(
            leaf.find_insert_pos(&20, &Rid::new(1, 4), &U64Comparator),
            3
        );
    }
}
