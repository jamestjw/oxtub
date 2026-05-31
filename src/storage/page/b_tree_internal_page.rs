use std::marker::PhantomData;

use crate::{
    common::{
        alignment::{align_up, max_usize},
        types::PageId,
    },
    storage::{
        disk::config::DEFAULT_PAGE_SIZE,
        index::comparator::KeyComparator,
        page::b_tree_node_header::{BTreeNodeHeader, PAGE_TYPE_INTERNAL},
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

pub struct BTreeInternalPage<'a, K, const PAGE_SIZE: usize = DEFAULT_PAGE_SIZE> {
    data: &'a [u8; PAGE_SIZE],
    _marker: PhantomData<K>,
}

pub struct BTreeInternalPageMut<'a, K, const PAGE_SIZE: usize = DEFAULT_PAGE_SIZE> {
    data: &'a mut [u8; PAGE_SIZE],
    _marker: PhantomData<K>,
}

type BTreeInternalHeader = BTreeNodeHeader;

impl<'a, K: bytemuck::Pod, const PAGE_SIZE: usize> BTreeInternalPageMut<'a, K, PAGE_SIZE> {
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

        while Self::values_end_for_slots(slots + 1) <= PAGE_SIZE {
            slots += 1;
        }

        slots
    }

    fn header(&self) -> &BTreeNodeHeader {
        let header_bytes = &self.data[..size_of::<BTreeInternalHeader>()];
        bytemuck::from_bytes(header_bytes)
    }

    fn header_mut(&mut self) -> &mut BTreeInternalHeader {
        let header_bytes = &mut self.data[..size_of::<BTreeInternalHeader>()];
        bytemuck::from_bytes_mut(header_bytes)
    }

    pub fn from_data(data: &'a mut [u8; PAGE_SIZE]) -> Self {
        Self {
            data,
            _marker: PhantomData,
        }
    }

    pub fn init(data: &'a mut [u8; PAGE_SIZE]) -> Self {
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
        self.values()[..self.curr_size()]
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

    pub fn insert_after(&mut self, after: &PageId, key: K, rid: Rid, val: PageId) {
        let after_idx = self.value_idx(after).expect("existing child ptr not found");
        let size = self.curr_size();
        let insert_idx = after_idx + 1;

        assert!(size < self.max_size());

        for i in (insert_idx..size).rev() {
            let key = *self.key_at(i);
            let rid = *self.rid_at(i);
            let val = *self.value_at(i);
            self.set_index_key_at(i + 1, &key, &rid);
            self.set_value_at(i + 1, val);
        }

        self.set_index_key_at(insert_idx, &key, &rid);
        self.set_value_at(insert_idx, val);
        self.header_mut().current_size += 1;
    }

    pub fn remove_at(&mut self, idx: usize) {
        let size = self.curr_size();
        assert!(idx < size);

        for i in idx..(size - 1) {
            let key = *self.key_at(i + 1);
            let rid = *self.rid_at(i + 1);
            let val = *self.value_at(i + 1);
            self.set_index_key_at(i, &key, &rid);
            self.set_value_at(i, val);
        }

        self.header_mut().current_size -= 1;
    }

    pub fn split_to(&mut self, recipient: &mut Self) -> (K, Rid) {
        let size = self.curr_size();
        let split_idx = self.min_size();
        assert!(size > split_idx);
        assert_eq!(recipient.curr_size(), 0);

        let promoted_key = *self.key_at(split_idx);
        let promoted_rid = *self.rid_at(split_idx);

        recipient.set_value_at(0, *self.value_at(split_idx));

        let mut recipient_size = 1;

        for i in (split_idx + 1)..size {
            let key = *self.key_at(i);
            let rid = *self.rid_at(i);
            let val = *self.value_at(i);
            recipient.set_index_key_at(recipient_size, &key, &rid);
            recipient.set_value_at(recipient_size, val);
            recipient_size += 1;
        }

        recipient.header_mut().set_size(recipient_size);
        self.header_mut().set_size(split_idx);

        (promoted_key, promoted_rid)
    }

    pub fn min_size(&self) -> usize {
        (self.max_size() + 1) / 2
    }

    // void RemoveAt(int index);
    // auto SplitTo(BPlusTreeInternalPage *recipient) -> KeyType;
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use expect_test::expect;

    use super::*;

    struct U64Comparator;

    impl KeyComparator<u64> for U64Comparator {
        fn compare(&self, a: &u64, b: &u64) -> Ordering {
            a.cmp(b)
        }
    }

    fn set_size<const PAGE_SIZE: usize>(
        page: &mut BTreeInternalPageMut<'_, u64, PAGE_SIZE>,
        size: usize,
    ) {
        page.header_mut().current_size = size as u16;
    }

    fn draw_internal_page<const PAGE_SIZE: usize>(
        page: &BTreeInternalPageMut<'_, u64, PAGE_SIZE>,
    ) -> String {
        let mut out = String::new();

        out.push_str(&format!(
            "size={}, max_size={}\n",
            page.curr_size(),
            page.max_size()
        ));

        for idx in 0..page.curr_size() {
            if idx == 0 {
                out.push_str(&format!(
                    "slot {idx}: key=<invalid>, value={}\n",
                    page.value_at(idx)
                ));
            } else {
                let rid = page.rid_at(idx);
                out.push_str(&format!(
                    "slot {idx}: key=({}, rid={}:{}), value={}\n",
                    page.key_at(idx),
                    rid.page_id,
                    rid.slot_id,
                    page.value_at(idx)
                ));
            }
        }

        out
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

    #[test]
    fn insert_after_shifts_entries_when_inserting_after_first_middle_and_last_child() {
        let mut data = [0; DEFAULT_PAGE_SIZE];
        let mut page = BTreeInternalPageMut::<u64>::init(&mut data);

        set_size(&mut page, 3);
        page.set_value_at(0, 100);
        page.set_index_key_at(1, &20, &Rid::new(2, 1));
        page.set_value_at(1, 102);
        page.set_index_key_at(2, &30, &Rid::new(3, 1));
        page.set_value_at(2, 103);

        expect![[r#"
size=3, max_size=409
slot 0: key=<invalid>, value=100
slot 1: key=(20, rid=2:1), value=102
slot 2: key=(30, rid=3:1), value=103
"#]]
        .assert_eq(&draw_internal_page(&page));

        page.insert_after(&100, 10, Rid::new(1, 1), 101);

        expect![[r#"
size=4, max_size=409
slot 0: key=<invalid>, value=100
slot 1: key=(10, rid=1:1), value=101
slot 2: key=(20, rid=2:1), value=102
slot 3: key=(30, rid=3:1), value=103
"#]]
        .assert_eq(&draw_internal_page(&page));

        page.insert_after(&101, 15, Rid::new(1, 5), 150);

        expect![[r#"
size=5, max_size=409
slot 0: key=<invalid>, value=100
slot 1: key=(10, rid=1:1), value=101
slot 2: key=(15, rid=1:5), value=150
slot 3: key=(20, rid=2:1), value=102
slot 4: key=(30, rid=3:1), value=103
"#]]
        .assert_eq(&draw_internal_page(&page));

        page.insert_after(&103, 40, Rid::new(4, 1), 104);

        expect![[r#"
size=6, max_size=409
slot 0: key=<invalid>, value=100
slot 1: key=(10, rid=1:1), value=101
slot 2: key=(15, rid=1:5), value=150
slot 3: key=(20, rid=2:1), value=102
slot 4: key=(30, rid=3:1), value=103
slot 5: key=(40, rid=4:1), value=104
"#]]
        .assert_eq(&draw_internal_page(&page));
    }

    #[test]
    #[should_panic(expected = "existing child ptr not found")]
    fn insert_after_panics_when_child_pointer_is_missing() {
        let mut data = [0; DEFAULT_PAGE_SIZE];
        let mut page = BTreeInternalPageMut::<u64>::init(&mut data);

        set_size(&mut page, 1);
        page.set_value_at(0, 100);

        page.insert_after(&999, 10, Rid::new(1, 1), 101);
    }

    #[test]
    #[should_panic]
    fn insert_after_panics_when_page_is_full() {
        let mut data = [0; DEFAULT_PAGE_SIZE];
        let mut page = BTreeInternalPageMut::<u64>::init(&mut data);

        let max_size = page.max_size();
        set_size(&mut page, max_size);
        page.set_value_at(0, 100);
        for idx in 1..max_size {
            page.set_index_key_at(idx, &(idx as u64 * 10), &Rid::new(idx, 1));
            page.set_value_at(idx, 100 + idx as PageId);
        }

        page.insert_after(&100, 5, Rid::new(0, 1), 101);
    }

    #[test]
    fn split_to_promotes_middle_key_and_moves_right_half_to_recipient() {
        let mut source_data = [0; 128];
        let mut recipient_data = [0; 128];
        let mut source = BTreeInternalPageMut::<u64, 128>::init(&mut source_data);
        let mut recipient = BTreeInternalPageMut::<u64, 128>::init(&mut recipient_data);

        let max_size = source.max_size();
        assert_eq!(max_size, 6);

        set_size(&mut source, max_size);
        source.set_value_at(0, 100);
        for idx in 1..max_size {
            source.set_index_key_at(idx, &(idx as u64 * 10), &Rid::new(idx, 1));
            source.set_value_at(idx, 100 + idx as PageId);
        }

        expect![[r#"
size=6, max_size=6
slot 0: key=<invalid>, value=100
slot 1: key=(10, rid=1:1), value=101
slot 2: key=(20, rid=2:1), value=102
slot 3: key=(30, rid=3:1), value=103
slot 4: key=(40, rid=4:1), value=104
slot 5: key=(50, rid=5:1), value=105
"#]]
        .assert_eq(&draw_internal_page(&source));

        let (promoted_key, promoted_rid) = source.split_to(&mut recipient);

        assert_eq!(promoted_key, 30);
        assert_eq!(promoted_rid, Rid::new(3, 1));

        expect![[r#"
size=3, max_size=6
slot 0: key=<invalid>, value=100
slot 1: key=(10, rid=1:1), value=101
slot 2: key=(20, rid=2:1), value=102
"#]]
        .assert_eq(&draw_internal_page(&source));

        expect![[r#"
size=3, max_size=6
slot 0: key=<invalid>, value=103
slot 1: key=(40, rid=4:1), value=104
slot 2: key=(50, rid=5:1), value=105
"#]]
        .assert_eq(&draw_internal_page(&recipient));
    }

    #[test]
    fn remove_at_shifts_entries_left_when_removing_first_middle_and_last_slot() {
        let mut data = [0; DEFAULT_PAGE_SIZE];
        let mut page = BTreeInternalPageMut::<u64>::init(&mut data);

        set_size(&mut page, 6);
        page.set_value_at(0, 100);
        for idx in 1..6 {
            page.set_index_key_at(idx, &(idx as u64 * 10), &Rid::new(idx, 1));
            page.set_value_at(idx, 100 + idx as PageId);
        }

        expect![[r#"
size=6, max_size=409
slot 0: key=<invalid>, value=100
slot 1: key=(10, rid=1:1), value=101
slot 2: key=(20, rid=2:1), value=102
slot 3: key=(30, rid=3:1), value=103
slot 4: key=(40, rid=4:1), value=104
slot 5: key=(50, rid=5:1), value=105
"#]]
        .assert_eq(&draw_internal_page(&page));

        page.remove_at(0);

        expect![[r#"
size=5, max_size=409
slot 0: key=<invalid>, value=101
slot 1: key=(20, rid=2:1), value=102
slot 2: key=(30, rid=3:1), value=103
slot 3: key=(40, rid=4:1), value=104
slot 4: key=(50, rid=5:1), value=105
"#]]
        .assert_eq(&draw_internal_page(&page));

        page.remove_at(2);

        expect![[r#"
size=4, max_size=409
slot 0: key=<invalid>, value=101
slot 1: key=(20, rid=2:1), value=102
slot 2: key=(40, rid=4:1), value=104
slot 3: key=(50, rid=5:1), value=105
"#]]
        .assert_eq(&draw_internal_page(&page));

        page.remove_at(3);

        expect![[r#"
size=3, max_size=409
slot 0: key=<invalid>, value=101
slot 1: key=(20, rid=2:1), value=102
slot 2: key=(40, rid=4:1), value=104
"#]]
        .assert_eq(&draw_internal_page(&page));
    }
}
