use std::mem::{align_of, size_of};

use crate::{
    buffer::page::{PageBytes, PageData},
    common::alignment::align_up,
    storage::{disk::config::DEFAULT_PAGE_SIZE, table::tuple::Tuple},
};

/**
 * Page to hold the intermediate data for external merge sort and hash join.
 * Supports variable-length tuples.
 *
 * Page layout:
 *
 *   byte offset
 *   0                                                                    DEFAULT_PAGE_SIZE
 *   +----------------+----------------------+------------+---------------------------+
 *   | num_tuples     | TupleInfo[0..n - 1]  | free space | tuple data                |
 *   | u16            | {offset, size}       |            | grows backward from end   |
 *   +----------------+----------------------+------------+---------------------------+
 *                    ^ grows forward                      ^ next tuple inserted here
 *
 * TupleInfo[i].offset points to the first byte of tuple i in the tuple data region.
 * TupleInfo[i].size stores the tuple's byte length.
 */

struct IntermediateResultPageView<'a> {
    data: &'a PageData,
}

pub struct IntermediateResultPage<'a> {
    view: IntermediateResultPageView<'a>,
}

pub struct IntermediateResultPageMut<'a> {
    data: &'a mut PageData,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TupleInfo {
    tuple_offset: u16,
    tuple_size: u16,
    // 4 bytes
}

impl TupleInfo {
    pub fn new(tuple_offset: usize, tuple_size: usize) -> Self {
        // DEFAULT_PAGE_SIZE is 8192, so tuple offsets and sizes always fit in u16.
        Self {
            tuple_offset: tuple_offset as u16,
            tuple_size: tuple_size as u16,
        }
    }
}

impl<'a> IntermediateResultPageView<'a> {
    // contains just the number of tuples
    const HEADER_SIZE: usize = size_of::<u16>();
    const TUPLE_INFOS_OFFSET: usize = align_up(Self::HEADER_SIZE, align_of::<TupleInfo>());

    pub fn from_data(data: &'a PageData) -> Self {
        Self { data }
    }

    fn bytes(&self) -> &PageBytes {
        &self.data.0
    }

    fn num_tuples(&self) -> usize {
        let num_tuples_bytes = &self.bytes()[..size_of::<u16>()];
        let res: u16 = *bytemuck::from_bytes(num_tuples_bytes);
        res as usize
    }

    fn tuple_infos(&self) -> &[TupleInfo] {
        let start = Self::TUPLE_INFOS_OFFSET;
        let end = start + self.num_tuples() * size_of::<TupleInfo>();
        bytemuck::cast_slice(&self.bytes()[start..end])
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

    pub fn get_tuple(&self, slot_id: usize) -> Tuple {
        let tuple_info = self.tuple_infos()[slot_id];
        let start = tuple_info.tuple_offset as usize;
        let end = start + tuple_info.tuple_size as usize;

        Tuple::from_bytes(self.bytes()[start..end].to_vec())
    }

    pub fn all_tuples(&self) -> Vec<Tuple> {
        (0..self.num_tuples())
            .map(|i| {
                let tuple_info = self.tuple_infos()[i];
                let start = tuple_info.tuple_offset as usize;
                let end = start + tuple_info.tuple_size as usize;

                Tuple::from_bytes(self.bytes()[start..end].to_vec())
            })
            .collect()
    }
}

impl<'a> IntermediateResultPage<'a> {
    pub fn from_data(data: &'a PageData) -> Self {
        Self {
            view: IntermediateResultPageView::from_data(data),
        }
    }

    pub fn num_tuples(&self) -> usize {
        self.view.num_tuples()
    }

    pub fn get_tuple(&self, slot_id: usize) -> Tuple {
        self.view.get_tuple(slot_id)
    }
}

impl<'a> IntermediateResultPageMut<'a> {
    pub fn from_data(data: &'a mut PageData) -> Self {
        Self { data }
    }

    pub fn init(&mut self) {
        self.data.0.fill(0);
    }

    pub fn init_from_data(data: &'a mut PageData) -> Self {
        let mut page = Self::from_data(data);
        page.init();
        page
    }

    fn view(&self) -> IntermediateResultPageView<'_> {
        IntermediateResultPageView::from_data(&*self.data)
    }

    fn bytes_mut(&mut self) -> &mut PageBytes {
        &mut self.data.0
    }

    fn num_tuples(&self) -> usize {
        self.view().num_tuples()
    }

    fn num_tuples_mut(&mut self) -> &mut u16 {
        bytemuck::from_bytes_mut(&mut self.bytes_mut()[..size_of::<u16>()])
    }

    fn free_space_start(&self) -> usize {
        self.view().free_space_start()
    }

    fn free_space_end(&self) -> usize {
        self.view().free_space_end()
    }

    fn append_tuple_info(&mut self, tuple_info: TupleInfo) {
        let start = self.free_space_start();
        let end = start + size_of::<TupleInfo>();
        self.bytes_mut()[start..end].copy_from_slice(bytemuck::bytes_of(&tuple_info));
        *self.num_tuples_mut() += 1;
    }

    pub fn insert_tuple(&mut self, tuple: &Tuple) -> Option<u16> {
        let tuple_size = tuple.size();
        // Even empty tuples require one TupleInfo entry, so max slots is
        // (8192 - 2) / 4 = 2047, well below u16::MAX.
        let slot_id = self.num_tuples() as u16;

        if size_of::<TupleInfo>() + tuple_size <= self.free_space_end() - self.free_space_start() {
            let tuple_offset = self.free_space_end() - tuple_size;
            self.bytes_mut()[tuple_offset..tuple_offset + tuple_size].copy_from_slice(tuple.data());
            self.append_tuple_info(TupleInfo::new(tuple_offset, tuple_size));
            Some(slot_id)
        } else {
            None
        }
    }

    pub fn all_tuples(&self) -> Vec<Tuple> {
        self.view().all_tuples()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intermediate_result_page_layout_sizes_match_expected_format() {
        assert_eq!(IntermediateResultPageView::HEADER_SIZE, 2);
        assert_eq!(IntermediateResultPageView::TUPLE_INFOS_OFFSET, 2);
        assert_eq!(size_of::<TupleInfo>(), 4);
    }

    #[test]
    fn inserts_and_reads_tuples() {
        let mut data = PageData([0; DEFAULT_PAGE_SIZE]);
        let mut page = IntermediateResultPageMut::init_from_data(&mut data);
        let tuple0 = Tuple::from_bytes(vec![1, 2, 3, 4]);
        let tuple1 = Tuple::from_bytes(vec![9, 8, 7]);

        assert_eq!(page.insert_tuple(&tuple0), Some(0));
        assert_eq!(page.insert_tuple(&tuple1), Some(1));
        assert_eq!(page.num_tuples(), 2);
        assert_eq!(page.free_space_start(), 2 + 2 * size_of::<TupleInfo>());
        assert_eq!(
            page.free_space_end(),
            DEFAULT_PAGE_SIZE - tuple0.size() - tuple1.size()
        );

        drop(page);
        let page = IntermediateResultPage::from_data(&data);

        assert_eq!(page.num_tuples(), 2);
        assert_eq!(page.get_tuple(0).data(), tuple0.data());
        assert_eq!(page.get_tuple(1).data(), tuple1.data());
    }

    #[test]
    fn rejects_tuple_that_cannot_fit() {
        let mut data = PageData([0; DEFAULT_PAGE_SIZE]);
        let mut page = IntermediateResultPageMut::init_from_data(&mut data);
        let tuple = Tuple::from_bytes(vec![0; DEFAULT_PAGE_SIZE]);

        assert_eq!(page.insert_tuple(&tuple), None);
        assert_eq!(page.num_tuples(), 0);
    }

    #[test]
    fn insert_tuple_can_exactly_fill_free_space() {
        let mut data = PageData([0; DEFAULT_PAGE_SIZE]);
        let mut page = IntermediateResultPageMut::init_from_data(&mut data);
        let tuple_size = DEFAULT_PAGE_SIZE
            - IntermediateResultPageView::TUPLE_INFOS_OFFSET
            - size_of::<TupleInfo>();
        let tuple = Tuple::from_bytes(vec![42; tuple_size]);

        assert_eq!(page.insert_tuple(&tuple), Some(0));
        assert_eq!(page.free_space_start(), page.free_space_end());
        assert_eq!(page.insert_tuple(&Tuple::from_bytes(vec![])), None);
    }
}
