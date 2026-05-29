use std::marker::PhantomData;

use bytemuck::Pod;

use crate::{
    buffer::page::{INVALID_PAGE_ID, PageBytes},
    common::alignment::align_up,
    storage::page::b_tree_page_header::{BTreePageHeader, PAGE_TYPE_LEAF},
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
    const KEYS_OFFSET: usize = align_up(
        Self::TOMBSTONES_OFFSET + TOMB_CAP * size_of::<TombstoneIndex>(),
        align_of::<K>(),
    );

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

    fn tombstones(&self) -> &[TombstoneIndex] {
        let start = Self::TOMBSTONES_OFFSET;
        let end = start + TOMB_CAP * size_of::<TombstoneIndex>();
        bytemuck::cast_slice(&self.data[start..end])
    }

    fn keys(&self) -> &[K] {
        let start = Self::KEYS_OFFSET;
        let end = start + self.max_size() * size_of::<K>();
        bytemuck::cast_slice(&self.data[start..end])
    }

    pub fn get_tombstoned_keys(&self) -> Vec<K> {
        let keys = self.keys();
        self.tombstones()[..self.get_tombstone_count()]
            .iter()
            .map(|idx| keys[usize::from(*idx)])
            .collect()
    }
}

impl<'a, K: Pod, const TOMB_CAP: usize> BTreeLeafPageMut<'a, K, TOMB_CAP> {
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

    pub fn init(data: &'a mut PageBytes, max_size: usize) -> Self {
        data.fill(0);

        let mut page = Self::from_data(data);
        let header = page.header_mut();
        header.common.init(PAGE_TYPE_LEAF, max_size);
        header.next_page_id = INVALID_PAGE_ID as u32;
        header.num_tombstones = 0;
        header._reserved = 0;

        page
    }

    fn header_mut(&mut self) -> &mut BTreeLeafHeader {
        let header_bytes = &mut self.data[..size_of::<BTreeLeafHeader>()];
        bytemuck::from_bytes_mut(header_bytes)
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
}

#[cfg(test)]
mod tests {
    use crate::{buffer::page::INVALID_PAGE_ID, storage::disk::config::DEFAULT_PAGE_SIZE};

    use super::*;

    #[test]
    fn init_sets_leaf_header_defaults() {
        let mut data = [0xff; DEFAULT_PAGE_SIZE];

        {
            let _leaf = BTreeLeafPageMut::<u64, 8>::init(&mut data, 128);
        }

        let leaf = BTreeLeafPage::<u64, 8>::from_data(&data);
        assert!(leaf.header().common.is_leaf());
        assert_eq!(leaf.max_size(), 128);
        assert_eq!(leaf.get_next_page_id(), INVALID_PAGE_ID);
        assert_eq!(leaf.get_tombstone_count(), 0);
    }
}
