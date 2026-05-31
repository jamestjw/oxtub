use std::marker::PhantomData;

use crate::{
    buffer::page::PageBytes,
    common::{
        alignment::{align_up, max_usize},
        types::PageId,
    },
    storage::{
        disk::config::DEFAULT_PAGE_SIZE,
        index::comparator::KeyComparator,
        page::b_tree_page_header::{BTreePageHeader, PAGE_TYPE_INTERNAL},
        rid::Rid,
    },
};

// Internal separators are index keys: `(K, Rid)`, ordered lexicographically.
// Slot 0 has an invalid separator key but a valid child pointer. For slots i > 0,
// separator i is the lower bound for child i. In terms of full index keys:
// child i contains entries >= separator i and < separator i + 1, except child 0
// has no lower bound and the last child has no upper bound.
//
// Logical-key lookups intentionally ignore Rid and route to the child before the
// first separator whose K is >= the search key. This finds the leftmost child that
// could contain a matching logical key when duplicates span multiple children.
//
// Internal page format (keys are stored in increasing order):
//  ---------
// | HEADER |
//  ---------
//  ----------------------------------------------------------------
// | KEY(0)(INVALID) | KEY(1)     | KEY(2)      | ... | KEY(n - 1) |
//  ---------------------------------------------------------------
//  -------------------------------------------------------------------
// | PAGE_ID(0)      | PAGE_ID(1) | PAGE_ID(2) | ... | PAGE_ID(n - 1) |
//  ------------------------------------------------------------------

pub struct BTreeInternalPage<'a, K> {
    data: &'a [u8],
    _marker: PhantomData<K>,
}

pub struct BTreeInternalPageMut<'a, K> {
    data: &'a mut [u8],
    _marker: PhantomData<K>,
}

type BTreeInternalHeader = BTreePageHeader;

impl<'a, K: bytemuck::Pod> BTreeInternalPageMut<'a, K> {
    const NUM_SLOTS: usize = Self::max_slots();

    const INDEX_KEY_ALIGN: usize = max_usize(align_of::<K>(), align_of::<Rid>());
    const INDEX_KEY_RID_OFFSET: usize = align_up(size_of::<K>(), align_of::<Rid>());
    const INDEX_KEY_SIZE: usize = align_up(
        Self::INDEX_KEY_RID_OFFSET + size_of::<Rid>(),
        Self::INDEX_KEY_ALIGN,
    );
    const KEYS_OFFSET: usize = align_up(size_of::<BTreeInternalHeader>(), Self::INDEX_KEY_ALIGN);
    const KEYS_END: usize = Self::KEYS_OFFSET + Self::NUM_SLOTS * Self::INDEX_KEY_SIZE;
    const VALUES_OFFSET: usize = align_up(Self::KEYS_END, align_of::<PageId>());
    const VALUES_END: usize = Self::VALUES_OFFSET + Self::NUM_SLOTS * size_of::<PageId>();

    const fn values_end_for_slots(num_slots: usize) -> usize {
        let keys_offset = align_up(size_of::<BTreeInternalHeader>(), Self::INDEX_KEY_ALIGN);
        let keys_end = keys_offset + num_slots * Self::INDEX_KEY_SIZE;
        let values_offset = align_up(keys_end, align_of::<PageId>());
        values_offset + num_slots * size_of::<PageId>()
    }

    const fn max_slots() -> usize {
        let mut slots = 0;

        while Self::values_end_for_slots(slots + 1) <= DEFAULT_PAGE_SIZE {
            slots += 1;
        }

        slots
    }

    fn header(&self) -> &BTreePageHeader {
        let header_bytes = &self.data[..size_of::<BTreeInternalHeader>()];
        bytemuck::from_bytes(header_bytes)
    }

    fn header_mut(&mut self) -> &mut BTreeInternalHeader {
        let header_bytes = &mut self.data[..size_of::<BTreeInternalHeader>()];
        bytemuck::from_bytes_mut(header_bytes)
    }

    pub fn from_data(data: &'a mut PageBytes) -> Self {
        Self {
            data,
            _marker: PhantomData,
        }
    }

    pub fn init(data: &'a mut PageBytes) -> Self {
        data.fill(0);

        let mut page = Self::from_data(data);
        page.header_mut().init(PAGE_TYPE_INTERNAL, Self::NUM_SLOTS);

        page
    }

    pub fn max_size(&self) -> usize {
        self.header().max_size as usize
    }

    fn curr_size(&self) -> usize {
        self.header().current_size as usize
    }

    fn index_key_offset(idx: usize) -> usize {
        Self::KEYS_OFFSET + idx * Self::INDEX_KEY_SIZE
    }

    fn key_ref(&self, idx: usize) -> &K {
        let start = Self::index_key_offset(idx);
        let end = start + size_of::<K>();
        bytemuck::from_bytes(&self.data[start..end])
    }

    fn rid_ref(&self, idx: usize) -> &Rid {
        let start = Self::index_key_offset(idx) + Self::INDEX_KEY_RID_OFFSET;
        let end = start + size_of::<Rid>();
        bytemuck::from_bytes(&self.data[start..end])
    }

    fn values(&self) -> &[PageId] {
        bytemuck::cast_slice(&self.data[Self::VALUES_OFFSET..Self::VALUES_END])
    }

    fn values_mut(&mut self) -> &mut [PageId] {
        bytemuck::cast_slice_mut(&mut self.data[Self::VALUES_OFFSET..Self::VALUES_END])
    }

    pub fn key_at(&self, idx: usize) -> &K {
        assert!(idx < self.curr_size());
        self.key_ref(idx)
    }

    pub fn rid_at(&self, idx: usize) -> &Rid {
        assert!(idx < self.curr_size());
        self.rid_ref(idx)
    }

    pub fn set_index_key_at(&mut self, idx: usize, key: &K, rid: &Rid) {
        self.write_index_key(idx, key, rid);
    }

    fn write_index_key(&mut self, idx: usize, key: &K, rid: &Rid) {
        assert!(idx < self.max_size(), "out of bounds");

        let key_start = Self::index_key_offset(idx);
        let key_end = key_start + size_of::<K>();
        self.data[key_start..key_end].copy_from_slice(bytemuck::bytes_of(key));

        let rid_start = key_start + Self::INDEX_KEY_RID_OFFSET;
        let rid_end = rid_start + size_of::<Rid>();
        self.data[rid_start..rid_end].copy_from_slice(bytemuck::bytes_of(rid));
    }

    pub fn value_idx(&self, value: &PageId) -> Option<usize> {
        self.values()
            .iter()
            .enumerate()
            .find_map(|(idx, v)| if *v == *value { Some(idx) } else { None })
    }

    pub fn value_at(&self, idx: usize) -> &PageId {
        &self.values()[idx]
    }

    pub fn set_value_at(&mut self, idx: usize, value: PageId) {
        self.values_mut()[idx] = value;
    }

    pub fn find_child_idx<C>(&self, key: &K, c: &C) -> usize
    where
        C: KeyComparator<K>,
    {
        // Invariant:
        //      self.key_ref[left] < k <= self.key_ref[right]
        let size = self.curr_size();
        assert!(size > 0);

        let mut left = 0;
        let mut right = size;

        while right - left > 1 {
            let mid = left + ((right - left) / 2);
            debug_assert!(mid > 0);

            if c.compare(self.key_ref(mid), key).is_lt() {
                left = mid;
            } else {
                right = mid;
            }
        }

        // right is the smallest value such that the invariant hold, but
        // since the separators also include the RID, the page on the
        // immediate left could also contain keys with K
        right - 1
    }

    // auto FindChildIndex(const KeyType &key, const KeyComparator &comparator) const -> int;
    // void InsertAfter(const ValueType &old_value, const KeyType &new_key, const ValueType &new_value);
    // void RemoveAt(int index);
    // auto SplitTo(BPlusTreeInternalPage *recipient) -> KeyType;
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use super::*;

    struct U64Comparator;

    impl KeyComparator<u64> for U64Comparator {
        fn compare(&self, a: &u64, b: &u64) -> Ordering {
            a.cmp(b)
        }
    }

    fn set_size(page: &mut BTreeInternalPageMut<'_, u64>, size: usize) {
        page.header_mut().current_size = size as u16;
    }

    #[test]
    fn find_child_idx_returns_only_child_when_page_has_no_separator_keys() {
        let mut data = [0; DEFAULT_PAGE_SIZE];
        let mut page = BTreeInternalPageMut::<u64>::init(&mut data);
        let comparator = U64Comparator;

        set_size(&mut page, 1);
        page.set_value_at(0, 100);

        assert_eq!(page.find_child_idx(&0, &comparator), 0);
        assert_eq!(*page.value_at(page.find_child_idx(&0, &comparator)), 100);
        assert_eq!(page.find_child_idx(&50, &comparator), 0);
        assert_eq!(*page.value_at(page.find_child_idx(&50, &comparator)), 100);
    }

    #[test]
    fn find_child_idx_routes_to_left_child_for_matching_separator_key() {
        let mut data = [0; DEFAULT_PAGE_SIZE];
        let mut page = BTreeInternalPageMut::<u64>::init(&mut data);
        let comparator = U64Comparator;

        set_size(&mut page, 4);
        for idx in 0..4 {
            page.set_value_at(idx, 100 + idx as PageId);
        }
        page.set_index_key_at(1, &10, &Rid::new(1, 1));
        page.set_index_key_at(2, &20, &Rid::new(2, 1));
        page.set_index_key_at(3, &30, &Rid::new(3, 1));

        assert_eq!(page.find_child_idx(&10, &comparator), 0);
        assert_eq!(*page.value_at(page.find_child_idx(&10, &comparator)), 100);
        assert_eq!(page.find_child_idx(&20, &comparator), 1);
        assert_eq!(*page.value_at(page.find_child_idx(&20, &comparator)), 101);
        assert_eq!(page.find_child_idx(&30, &comparator), 2);
        assert_eq!(*page.value_at(page.find_child_idx(&30, &comparator)), 102);
    }

    #[test]
    fn find_child_idx_routes_between_distinct_separator_keys() {
        let mut data = [0; DEFAULT_PAGE_SIZE];
        let mut page = BTreeInternalPageMut::<u64>::init(&mut data);
        let comparator = U64Comparator;

        set_size(&mut page, 4);
        page.set_index_key_at(1, &10, &Rid::new(1, 1));
        page.set_index_key_at(2, &20, &Rid::new(2, 1));
        page.set_index_key_at(3, &30, &Rid::new(3, 1));

        assert_eq!(page.find_child_idx(&0, &comparator), 0);
        assert_eq!(page.find_child_idx(&9, &comparator), 0);
        assert_eq!(page.find_child_idx(&11, &comparator), 1);
        assert_eq!(page.find_child_idx(&19, &comparator), 1);
        assert_eq!(page.find_child_idx(&21, &comparator), 2);
        assert_eq!(page.find_child_idx(&29, &comparator), 2);
        assert_eq!(page.find_child_idx(&31, &comparator), 3);
        assert_eq!(page.find_child_idx(&u64::MAX, &comparator), 3);
    }

    #[test]
    fn find_child_idx_returns_leftmost_possible_child_for_duplicate_separator_keys() {
        let mut data = [0; DEFAULT_PAGE_SIZE];
        let mut page = BTreeInternalPageMut::<u64>::init(&mut data);
        let comparator = U64Comparator;

        set_size(&mut page, 6);
        page.set_index_key_at(1, &10, &Rid::new(1, 1));
        page.set_index_key_at(2, &20, &Rid::new(2, 1));
        page.set_index_key_at(3, &20, &Rid::new(2, 2));
        page.set_index_key_at(4, &20, &Rid::new(2, 3));
        page.set_index_key_at(5, &30, &Rid::new(3, 1));

        assert_eq!(page.find_child_idx(&9, &comparator), 0);
        assert_eq!(page.find_child_idx(&10, &comparator), 0);
        assert_eq!(page.find_child_idx(&11, &comparator), 1);
        assert_eq!(page.find_child_idx(&20, &comparator), 1);
        assert_eq!(page.find_child_idx(&21, &comparator), 4);
        assert_eq!(page.find_child_idx(&30, &comparator), 4);
        assert_eq!(page.find_child_idx(&31, &comparator), 5);
    }
}
