use std::mem::{align_of, size_of};

use crate::{
    buffer::page::{INVALID_PAGE_ID, PageBytes},
    common::{alignment::align_up, types::PageId},
    storage::{
        disk::config::DEFAULT_PAGE_SIZE,
        table::tuple::{Tuple, TupleMeta},
    },
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

    fn num_deleted_tuples(&self) -> usize {
        self.header().num_deleted_tuples as usize
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

    pub fn get_tuple(&self, slot_id: usize) -> (TupleMeta, Tuple) {
        let tuple_info = self.tuple_infos()[slot_id];
        let start = tuple_info.tuple_offset as usize;
        let end = start + tuple_info.tuple_size as usize;

        (
            tuple_info.tuple_meta,
            Tuple::from_bytes(self.data[start..end].to_vec()),
        )
    }
}

impl<'a> TablePage<'a> {
    pub fn from_data(data: &'a PageBytes) -> Self {
        Self {
            view: TablePageView { data },
        }
    }

    pub fn get_tuple(&self, slot_id: usize) -> (TupleMeta, Tuple) {
        self.view.get_tuple(slot_id)
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
        header.next_page_id = INVALID_PAGE_ID;
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

    fn free_space_end(&self) -> usize {
        self.view().free_space_end()
    }

    fn num_tuples(&self) -> usize {
        self.view().num_tuples()
    }

    fn num_deleted_tuples(&self) -> usize {
        self.view().num_deleted_tuples()
    }

    fn set_num_deleted_tuples(&mut self, val: usize) {
        self.header_mut().num_deleted_tuples = val as u16;
    }

    pub fn set_next_page_id(&mut self, page_id: PageId) {
        self.header_mut().next_page_id = page_id;
    }

    fn tuple_infos(&self) -> &[TupleInfo] {
        let start = TablePageView::TUPLE_INFOS_OFFSET;
        let end = start + self.view().num_tuples() * size_of::<TupleInfo>();
        bytemuck::cast_slice(&self.data[start..end])
    }

    fn tuple_infos_mut(&mut self) -> &mut [TupleInfo] {
        let start = TablePageView::TUPLE_INFOS_OFFSET;
        let end = start + self.view().num_tuples() * size_of::<TupleInfo>();
        bytemuck::cast_slice_mut(&mut self.data[start..end])
    }

    fn append_tuple_info(&mut self, tuple_info: TupleInfo) {
        let start = self.free_space_start();
        let end = start + size_of::<TupleInfo>();
        self.data[start..end].copy_from_slice(bytemuck::bytes_of(&tuple_info));
        self.header_mut().num_tuples += 1;
    }

    pub fn insert_tuple(&mut self, meta: &TupleMeta, tuple: &Tuple) -> Option<u16> {
        let tuple_size = tuple.size();
        let slot_id = self.num_tuples() as u16;
        if size_of::<TupleInfo>() + tuple_size <= self.free_space_end() - self.free_space_start() {
            let tuple_offset = self.free_space_end() - tuple_size;
            self.data[tuple_offset..tuple_offset + tuple_size].copy_from_slice(tuple.data());
            self.append_tuple_info(TupleInfo::new(tuple_offset, tuple_size, meta.clone()));

            if meta.is_deleted() {
                self.set_num_deleted_tuples(self.num_deleted_tuples() + 1);
            }

            Some(slot_id)
        } else {
            // Can't fit tuple on this page
            None
        }
    }

    pub fn update_tuple_meta(&mut self, slot_id: usize, meta: TupleMeta) {
        assert!(slot_id < self.num_tuples());
        let old_meta = self.tuple_infos()[slot_id].tuple_meta;
        self.tuple_infos_mut()[slot_id].tuple_meta = meta;

        if old_meta.is_deleted() && !meta.is_deleted() {
            self.set_num_deleted_tuples(self.num_deleted_tuples() - 1);
        } else if !old_meta.is_deleted() && meta.is_deleted() {
            self.set_num_deleted_tuples(self.num_deleted_tuples() + 1);
        }
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

    #[test]
    fn insert_tuple_writes_metadata_and_bytes() {
        let mut data = [0; DEFAULT_PAGE_SIZE];
        let mut page = TablePageMut::init(&mut data);

        let meta0 = TupleMeta::new(11, false);
        let tuple0 = Tuple::from_bytes(vec![1, 2, 3, 4]);
        let meta1 = TupleMeta::new(22, false);
        let tuple1 = Tuple::from_bytes(vec![9, 8, 7]);

        assert_eq!(page.insert_tuple(&meta0, &tuple0), Some(0));
        assert_eq!(page.insert_tuple(&meta1, &tuple1), Some(1));

        assert_eq!(page.num_tuples(), 2);
        assert_eq!(page.num_deleted_tuples(), 0);
        assert_eq!(
            page.free_space_start(),
            TablePageView::TUPLE_INFOS_OFFSET + 2 * size_of::<TupleInfo>()
        );
        assert_eq!(
            page.free_space_end(),
            DEFAULT_PAGE_SIZE - tuple0.size() - tuple1.size()
        );

        let tuple_infos = page.tuple_infos();
        assert_eq!(
            tuple_infos[0],
            TupleInfo::new(DEFAULT_PAGE_SIZE - tuple0.size(), tuple0.size(), meta0)
        );
        assert_eq!(
            tuple_infos[1],
            TupleInfo::new(
                DEFAULT_PAGE_SIZE - tuple0.size() - tuple1.size(),
                tuple1.size(),
                meta1
            )
        );

        let tuple0_offset = tuple_infos[0].tuple_offset as usize;
        let tuple1_offset = tuple_infos[1].tuple_offset as usize;
        assert_eq!(
            &page.data[tuple0_offset..tuple0_offset + tuple0.size()],
            tuple0.data()
        );
        assert_eq!(
            &page.data[tuple1_offset..tuple1_offset + tuple1.size()],
            tuple1.data()
        );
    }

    #[test]
    fn insert_tuple_tracks_deleted_count() {
        let mut data = [0; DEFAULT_PAGE_SIZE];
        let mut page = TablePageMut::init(&mut data);
        let tuple = Tuple::from_bytes(vec![1, 2, 3]);

        assert_eq!(page.insert_tuple(&TupleMeta::new(1, true), &tuple), Some(0));
        assert_eq!(page.num_deleted_tuples(), 1);

        page.update_tuple_meta(0, TupleMeta::new(2, false));
        assert_eq!(page.num_deleted_tuples(), 0);

        page.update_tuple_meta(0, TupleMeta::new(3, true));
        assert_eq!(page.num_deleted_tuples(), 1);
    }

    #[test]
    fn insert_tuple_returns_none_when_tuple_does_not_fit() {
        let mut data = [0; DEFAULT_PAGE_SIZE];
        let mut page = TablePageMut::init(&mut data);
        let tuple = Tuple::from_bytes(vec![0; DEFAULT_PAGE_SIZE]);

        assert_eq!(page.insert_tuple(&TupleMeta::new(1, false), &tuple), None);
        assert_eq!(page.num_tuples(), 0);
        assert_eq!(page.num_deleted_tuples(), 0);
        assert_eq!(page.free_space_start(), TablePageView::TUPLE_INFOS_OFFSET);
        assert_eq!(page.free_space_end(), DEFAULT_PAGE_SIZE);
    }

    #[test]
    fn insert_tuple_can_exactly_fill_free_space() {
        let mut data = [0; DEFAULT_PAGE_SIZE];
        let mut page = TablePageMut::init(&mut data);
        let tuple_size =
            DEFAULT_PAGE_SIZE - TablePageView::TUPLE_INFOS_OFFSET - size_of::<TupleInfo>();
        let tuple = Tuple::from_bytes(vec![42; tuple_size]);

        assert_eq!(
            page.insert_tuple(&TupleMeta::new(1, false), &tuple),
            Some(0)
        );
        assert_eq!(page.free_space_start(), page.free_space_end());
        assert_eq!(
            page.insert_tuple(&TupleMeta::new(2, false), &Tuple::from_bytes(vec![])),
            None
        );
    }
}
