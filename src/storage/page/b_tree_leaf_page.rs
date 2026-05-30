use std::marker::PhantomData;

use bytemuck::Pod;

use crate::{
    buffer::page::{INVALID_PAGE_ID, PageBytes},
    common::alignment::align_up,
    storage::{
        disk::config::DEFAULT_PAGE_SIZE,
        index::comparator::KeyComparator,
        page::b_tree_page_header::{BTreePageHeader, PAGE_TYPE_LEAF},
        rid::Rid,
    },
};

pub struct BTreeLeafPage<'a, K, const TOMB_CAP: usize> {
    data: &'a [u8],
    _marker: PhantomData<K>,
}

pub struct BTreeLeafPageMut<'a, K, const TOMB_CAP: usize> {
    data: &'a mut [u8],
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

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BTreeLeafHeader {
    common: BTreePageHeader,
    next_page_id: u32,
    num_tombstones: u16,
    _reserved: u16,
    // 16 bytes header
}

impl<'a, K: Pod + Copy, const TOMB_CAP: usize> BTreeLeafPage<'a, K, TOMB_CAP> {
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

    pub fn from_data(data: &'a PageBytes) -> Self {
        Self {
            data,
            _marker: PhantomData,
        }
    }

    fn header(&self) -> &BTreeLeafHeader {
        let header_bytes = &self.data[..size_of::<BTreeLeafHeader>()];
        bytemuck::from_bytes(header_bytes)
    }

    pub fn get_tombstone_count(&self) -> usize {
        self.header().num_tombstones as usize
    }

    pub fn get_next_page_id(&self) -> usize {
        self.header().next_page_id as usize
    }

    pub fn max_size(&self) -> usize {
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

    pub fn get_tombstoned_keys(&self) -> Vec<K> {
        self.tombstones()[..self.get_tombstone_count()]
            .iter()
            .map(|idx| *self.key_ref(usize::from(*idx)))
            .collect()
    }

    pub fn key_at(&self, idx: usize) -> &K {
        assert!(idx < self.curr_size());
        self.key_ref(idx)
    }

    pub fn value_at(&self, idx: usize) -> &Rid {
        assert!(idx < self.curr_size());
        self.rid_ref(idx)
    }
}

impl<'a, K: Pod, const TOMB_CAP: usize> BTreeLeafPageMut<'a, K, TOMB_CAP> {
    pub const MAX_SIZE: usize = leaf_page_max_size::<K, TOMB_CAP>();
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

    // fn header(&self) -> &BTreeLeafHeader {
    //     let header_bytes = &self.data[..size_of::<BTreeLeafHeader>()];
    //     bytemuck::from_bytes(header_bytes)
    // }
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

    fn header(&self) -> &BTreeLeafHeader {
        let header_bytes = &self.data[..size_of::<BTreeLeafHeader>()];
        bytemuck::from_bytes(header_bytes)
    }

    fn header_mut(&mut self) -> &mut BTreeLeafHeader {
        let header_bytes = &mut self.data[..size_of::<BTreeLeafHeader>()];
        bytemuck::from_bytes_mut(header_bytes)
    }

    pub fn max_size(&self) -> usize {
        self.header().common.max_size as usize
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

    // pub fn get_tombstone_count(&self) -> usize {
    //     self.header().num_tombstones as usize
    // }
    //
    // pub fn get_next_page_id(&self) -> usize {
    //     self.header().next_page_id as usize
    // }

    pub fn set_next_page_id(&mut self, page_id: usize) {
        assert!(page_id <= u32::MAX as usize);

        self.header_mut().next_page_id = page_id as u32;
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

    pub fn find_pos<C>(&self, key: &K, c: &C) -> usize
    where
        C: KeyComparator<K>,
    {
        self.lower_bound_by(|idx| c.compare(self.key_ref(idx), key))
    }

    pub fn find_insert_pos<C>(&self, key: &K, rid: &Rid, c: &C) -> usize
    where
        C: KeyComparator<K>,
    {
        self.lower_bound_by(|idx| {
            c.compare(self.key_ref(idx), key)
                .then_with(|| compare_rid(self.rid_ref(idx), rid))
        })
    }
}

fn compare_rid(a: &Rid, b: &Rid) -> std::cmp::Ordering {
    a.page_id
        .cmp(&b.page_id)
        .then_with(|| a.slot_id.cmp(&b.slot_id))
}

const fn max_usize(a: usize, b: usize) -> usize {
    if a > b { a } else { b }
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

    use crate::{buffer::page::INVALID_PAGE_ID, storage::disk::config::DEFAULT_PAGE_SIZE};

    use super::*;

    struct U64Comparator;

    impl KeyComparator<u64> for U64Comparator {
        fn compare(&self, a: &u64, b: &u64) -> Ordering {
            a.cmp(b)
        }
    }

    #[test]
    fn init_sets_leaf_header_defaults() {
        let mut data = [0xff; DEFAULT_PAGE_SIZE];

        {
            let _leaf = BTreeLeafPageMut::<u64, 8>::init(&mut data);
        }

        let leaf = BTreeLeafPage::<u64, 8>::from_data(&data);
        assert!(leaf.header().common.is_leaf());
        assert_eq!(leaf.max_size(), BTreeLeafPageMut::<u64, 8>::MAX_SIZE);
        assert_eq!(leaf.get_next_page_id(), INVALID_PAGE_ID);
        assert_eq!(leaf.get_tombstone_count(), 0);
    }

    fn write_entry<K: Pod, const TOMB_CAP: usize>(
        leaf: &mut BTreeLeafPageMut<'_, K, TOMB_CAP>,
        idx: usize,
        key: K,
        rid: Rid,
    ) {
        let key_start = BTreeLeafPageMut::<K, TOMB_CAP>::entry_offset(idx);
        let key_end = key_start + size_of::<K>();
        leaf.data[key_start..key_end].copy_from_slice(bytemuck::bytes_of(&key));

        let rid_start = key_start + BTreeLeafPageMut::<K, TOMB_CAP>::ENTRY_RID_OFFSET;
        let rid_end = rid_start + size_of::<Rid>();
        leaf.data[rid_start..rid_end].copy_from_slice(bytemuck::bytes_of(&rid));
    }

    #[test]
    fn full_page_entries_round_trip() {
        let mut data = [0; DEFAULT_PAGE_SIZE];
        let max_size = BTreeLeafPageMut::<u32, 8>::MAX_SIZE;

        {
            let mut leaf = BTreeLeafPageMut::<u32, 8>::init(&mut data);
            leaf.header_mut().common.current_size = max_size as u16;

            for idx in 0..max_size {
                write_entry(&mut leaf, idx, idx as u32 + 100, Rid::new(idx + 1, idx));
            }
        }

        let leaf = BTreeLeafPage::<u32, 8>::from_data(&data);

        for idx in 0..max_size {
            assert_eq!(*leaf.key_at(idx), idx as u32 + 100);
            assert_eq!(*leaf.value_at(idx), Rid::new(idx + 1, idx));
        }
    }

    #[test]
    fn find_pos_returns_first_matching_logical_key() {
        let mut data = [0; DEFAULT_PAGE_SIZE];
        let mut leaf = BTreeLeafPageMut::<u64, 8>::init(&mut data);
        leaf.header_mut().common.current_size = 5;
        write_entry(&mut leaf, 0, 10, Rid::new(1, 1));
        write_entry(&mut leaf, 1, 20, Rid::new(1, 1));
        write_entry(&mut leaf, 2, 20, Rid::new(1, 2));
        write_entry(&mut leaf, 3, 30, Rid::new(1, 1));
        write_entry(&mut leaf, 4, 40, Rid::new(1, 1));

        assert_eq!(leaf.find_pos(&20, &U64Comparator), 1);
        assert_eq!(leaf.find_pos(&25, &U64Comparator), 3);
        assert_eq!(leaf.find_pos(&30, &U64Comparator), 3);
    }

    #[test]
    fn find_insert_pos_uses_rid_as_tiebreaker() {
        let mut data = [0; DEFAULT_PAGE_SIZE];
        let mut leaf = BTreeLeafPageMut::<u64, 8>::init(&mut data);
        leaf.header_mut().common.current_size = 4;
        write_entry(&mut leaf, 0, 10, Rid::new(1, 1));
        write_entry(&mut leaf, 1, 20, Rid::new(1, 1));
        write_entry(&mut leaf, 2, 20, Rid::new(1, 3));
        write_entry(&mut leaf, 3, 30, Rid::new(1, 1));

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
