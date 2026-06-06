use std::marker::PhantomData;

use crate::{
    buffer::{bpm::BufferPoolManager, page::INVALID_PAGE_ID, page_guard::ReadPageGuard},
    storage::{page::b_tree_leaf_page::BTreeLeafPage, rid::Rid},
};

pub struct IndexIterator<'a, K, const TOMB_CAP: usize> {
    bpm: &'a BufferPoolManager,
    read_page_guard: Option<ReadPageGuard<'a>>,
    idx: usize,
    _marker: PhantomData<K>,
}

impl<'a, K: bytemuck::Pod + Copy, const TOMB_CAP: usize> IndexIterator<'a, K, TOMB_CAP> {
    pub fn new(bpm: &'a BufferPoolManager, read_page_guard: Option<ReadPageGuard<'a>>) -> Self {
        Self {
            bpm,
            read_page_guard,
            idx: 0,
            _marker: PhantomData,
        }
    }
}

impl<'a, K: bytemuck::Pod + Copy, const TOMB_CAP: usize> Iterator
    for IndexIterator<'a, K, TOMB_CAP>
{
    type Item = (K, Rid);

    fn next(&mut self) -> Option<Self::Item> {
        if self.read_page_guard.is_none() {
            return None;
        }

        loop {
            let curr_leaf = BTreeLeafPage::<K, TOMB_CAP>::from_data(
                self.read_page_guard.as_ref().unwrap().data(),
            );

            while self.idx < curr_leaf.curr_size() {
                let idx = self.idx;
                self.idx += 1;

                if curr_leaf.is_idx_tombstoned(idx) {
                    continue;
                }

                return Some((*curr_leaf.key_at(idx), *curr_leaf.value_at(idx)));
            }

            let next_page_id = curr_leaf.get_next_page_id();
            if next_page_id == INVALID_PAGE_ID {
                self.read_page_guard = None;
                return None;
            }

            self.read_page_guard.take();
            self.read_page_guard = Some(self.bpm.read_page(next_page_id).unwrap());
            self.idx = 0;
        }
    }
}
