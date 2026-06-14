use std::mem::{align_of, size_of};

use crate::{
    buffer::page::PageBytes,
    common::alignment::align_up,
    storage::{disk::config::DEFAULT_PAGE_SIZE, table::tuple::TupleMeta},
};

// byte offset
// 0
// |
// v
// +----------------------+--------------------+----------------+----------------+
// | TablePage header     | tuple_info array   | free space     | tuple bytes    |
// | fixed size: 8 bytes  | grows forward      | unused bytes   | grows backward |
// +----------------------+--------------------+----------------+----------------+
// ^                      ^                    ^                ^                ^
// |                      |                    |                |                |
// page_start             1st slot metadata    slot array end   next tuple insert DEFAULT_PAGE_SIZE
//
//
// Header contents:
// TablePage header, 8 bytes total
//
// +---------------------------+---------------------------+---------------------------+
// | next_page_id              | num_tuples                | num_deleted_tuples        |
// | 4 bytes                   | 2 bytes                   | 2 bytes                   |
// +---------------------------+---------------------------+---------------------------+
//
// Each tuple_info entry:
// tuple_infos[i], 24 bytes total
//
// +---------------------------+---------------------------+---------------------------+
// | tuple_offset              | tuple_size                | padding                   |
// | 2 bytes                   | 2 bytes                   | 4 bytes                   |
// +---------------------------+---------------------------+---------------------------+
// | TupleMeta                                                                 |
// | 16 bytes                                                                  |
// +---------------------------+---------------------------+---------------------------+
//
// TupleMeta, 16 bytes total
//
// byte offset
// 0                                                           16
// |                                                            |
// v                                                            v
// +--------------------------------+-------------+-------------+
// | txn_id                         | is_deleted  | padding     |
// | u64                            | u8          | unused      |
// | 8 bytes                        | 1 byte      | 7 bytes     |
// +--------------------------------+-------------+-------------+
// 0                                8             9             16
struct TablePageView<'a> {
    data: &'a PageBytes,
}

pub struct TablePage<'a> {
    view: TablePageView<'a>,
}

pub struct TablePageMut<'a> {
    data: &'a mut PageBytes,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TablePageHeader {
    next_page_id: u32,
    num_tuples: u16,
    num_deleted_tuples: u16,
    // 8 bytes header
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TupleInfo {
    tuple_offset: u16,
    tuple_size: u16,
    _padding: [u8; 4],
    tuple_meta: TupleMeta,
    // 24 bytes header
}

impl TupleInfo {
    pub fn new(tuple_offset: usize, tuple_size: usize, tuple_meta: TupleMeta) -> Self {
        Self {
            tuple_offset: tuple_offset as u16,
            tuple_size: tuple_size as u16,
            _padding: [0; 4],
            tuple_meta,
        }
    }
}

impl<'a> TablePageView<'a> {
    const HEADER_SIZE: usize = size_of::<TablePageHeader>();
    const TUPLE_INFOS_OFFSET: usize = align_up(Self::HEADER_SIZE, align_of::<TupleInfo>());

    pub fn from_data(data: &'a PageBytes) -> Self {
        Self { data }
    }

    fn header(&self) -> &TablePageHeader {
        let header_bytes = &self.data[..size_of::<TablePageHeader>()];
        bytemuck::from_bytes(header_bytes)
    }

    fn num_tuples(&self) -> usize {
        self.header().num_tuples as usize
    }

    fn tuple_infos(&self) -> &[TupleInfo] {
        let start = Self::TUPLE_INFOS_OFFSET;
        let end = start + self.num_tuples() * size_of::<TupleInfo>();
        bytemuck::cast_slice(&self.data[start..end])
    }

    // Offset from which is unoccupied (inclusive)
    fn free_space_start(&self) -> usize {
        Self::TUPLE_INFOS_OFFSET + self.num_tuples() * size_of::<TupleInfo>()
    }

    // Offset until which is unoccupied (exclusive)
    //
    // Tuple infos are appended left-to-right while tuple bytes grow right-to-left,
    // so the last tuple info points to the current start of tuple storage.
    fn free_space_end(&self) -> usize {
        let num_tuples = self.num_tuples();

        if num_tuples == 0 {
            DEFAULT_PAGE_SIZE
        } else {
            let last_tuple_info = self.tuple_infos()[num_tuples - 1];
            last_tuple_info.tuple_offset as usize
        }
    }
}

impl<'a> TablePageMut<'a> {
    pub fn from_data(data: &'a mut PageBytes) -> Self {
        Self { data }
    }

    pub fn init(data: &'a mut PageBytes) -> Self {
        data.fill(0);

        let mut page = Self::from_data(data);
        let header = page.header_mut();
        header.num_tuples = 0;
        header.num_deleted_tuples = 0;

        page
    }

    fn view(&self) -> TablePageView<'_> {
        TablePageView::from_data(&*self.data)
    }

    fn header_mut(&mut self) -> &mut TablePageHeader {
        let header_bytes = &mut self.data[..size_of::<TablePageHeader>()];
        bytemuck::from_bytes_mut(header_bytes)
    }

    fn free_space_start(&self) -> usize {
        self.view().free_space_start()
    }

    fn tuple_infos(&self) -> &[TupleInfo] {
        let start = TablePageView::TUPLE_INFOS_OFFSET;
        let end = start + self.view().num_tuples() * size_of::<TupleInfo>();
        bytemuck::cast_slice(&self.data[start..end])
    }

    pub fn append_tuple_info(&mut self, tuple_info: TupleInfo) {
        let start = self.free_space_start();
        let end = start + size_of::<TupleInfo>();
        self.data[start..end].copy_from_slice(bytemuck::bytes_of(&tuple_info));
        self.header_mut().num_tuples += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_page_layout_sizes_match_expected_format() {
        assert_eq!(size_of::<TablePageHeader>(), 8);
        assert_eq!(size_of::<TupleMeta>(), 16);
        assert_eq!(size_of::<TupleInfo>(), 24);
    }

    #[test]
    fn tuple_infos_start_after_aligned_header() {
        assert_eq!(TablePageView::HEADER_SIZE, 8);
        assert_eq!(TablePageView::TUPLE_INFOS_OFFSET, 8);
    }

    #[test]
    fn test_tuple_info_offsets() {
        // If we can write and then read stuff correctly, it means that
        // nothing is getting clobberred and hence the offsets are right.
        fn build_tuple_info(idx: usize) -> TupleInfo {
            TupleInfo::new(idx, idx * 7, TupleMeta::new(idx * 29, idx % 2 == 0))
        }

        let mut data = [0; DEFAULT_PAGE_SIZE];
        let mut page = TablePageMut::init(&mut data);

        for idx in 0..100 {
            page.append_tuple_info(build_tuple_info(idx));
        }

        for idx in 0..100 {
            let tuple_info = page.tuple_infos()[idx];
            assert_eq!(tuple_info, build_tuple_info(idx));
        }
    }
}
