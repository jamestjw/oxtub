use std::sync::Mutex;

use crate::{
    buffer::{bpm::BufferPoolManager, page::INVALID_PAGE_ID, page_guard::ReadPageGuard},
    common::types::PageId,
    storage::{
        page::table_page::{TablePage, TablePageMut},
        rid::Rid,
        table::{
            error::TableHeapError,
            tuple::{Tuple, TupleMeta},
        },
    },
};

pub struct TableHeap<'a> {
    bpm: &'a BufferPoolManager,
    inner: Mutex<TableHeapInner>,
}

struct TableHeapInner {
    first_page_id: PageId,
    last_page_id: PageId,
}

impl<'a> TableHeap<'a> {
    pub fn new(bpm: &'a BufferPoolManager) -> Result<Self, TableHeapError> {
        let first_page_id = bpm.new_page();
        let mut page_guard = bpm.write_page(first_page_id)?;
        TablePageMut::init(page_guard.data_mut());

        Ok(Self {
            bpm,
            inner: Mutex::new(TableHeapInner {
                first_page_id,
                last_page_id: first_page_id,
            }),
        })
    }

    pub fn insert_tuple(&self, meta: &TupleMeta, tuple: &Tuple) -> Result<Rid, TableHeapError> {
        let mut inner = self.inner.lock().unwrap();

        // Small optimisation to only use a single frame for insertions
        let slot_id = {
            let mut page_guard = self.bpm.write_page(inner.last_page_id)?;
            let mut page = TablePageMut::from_data(page_guard.data_mut());
            page.insert_tuple(meta, tuple)
        };

        let (page_id, slot_id) = match slot_id {
            Some(slot_id) => (inner.last_page_id, slot_id as usize),
            None => {
                // Ok no space here in the last page, let's try getting a new page
                // and inserting there
                let (new_page_id, slot_id) = {
                    let new_page_id = self.bpm.new_page();
                    let mut new_page_guard = self.bpm.write_page(new_page_id)?;
                    let mut new_page = TablePageMut::init(new_page_guard.data_mut());
                    let res = new_page.insert_tuple(meta, tuple);
                    drop(new_page_guard);
                    (new_page_id, res)
                };

                match slot_id {
                    None => {
                        self.bpm.delete_page(new_page_id)?;

                        // TODO: eventually we should support saving tuples larger than 1 page
                        // across multiple pages or something like that
                        return Err(TableHeapError::TupleTooLarge);
                    }
                    Some(slot_id) => {
                        let mut page_guard = self.bpm.write_page(inner.last_page_id)?;
                        TablePageMut::from_data(page_guard.data_mut())
                            .set_next_page_id(new_page_id);
                        inner.last_page_id = new_page_id;
                        (new_page_id, slot_id as usize)
                    }
                }
            }
        };

        Ok(Rid::new(page_id, slot_id))
    }

    pub fn get_tuple(&self, rid: Rid) -> Result<(TupleMeta, Tuple), TableHeapError> {
        let page_guard = self.bpm.read_page(rid.page_id)?;
        let page = TablePage::from_data(page_guard.data());
        Ok(page.get_tuple(rid.slot_id as usize))
    }

    pub fn iter(&self) -> TableHeapIterator<'_> {
        TableHeapIterator {
            bpm: self.bpm,
            idx: 0,
            read_page_guard: self
                .bpm
                .read_page(self.inner.lock().unwrap().first_page_id)
                .unwrap()
                .into(),
        }
    }
}

pub struct TableHeapIterator<'a> {
    bpm: &'a BufferPoolManager,
    idx: usize,
    read_page_guard: Option<ReadPageGuard<'a>>,
}

impl<'a> Iterator for TableHeapIterator<'a> {
    type Item = (Rid, TupleMeta, Tuple);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let guard = self.read_page_guard.as_ref()?;
            let curr_page = TablePage::from_data(guard.data());

            while self.idx < curr_page.num_tuples() {
                let idx = self.idx;
                self.idx += 1;

                let (meta, tuple) = curr_page.get_tuple(idx);

                let rid = Rid::new(guard.page_id(), idx);

                return Some((rid, meta, tuple));
            }

            let next_page_id = curr_page.next_page_id();
            self.read_page_guard.take();

            if next_page_id == INVALID_PAGE_ID {
                return None;
            }

            self.read_page_guard = Some(self.bpm.read_page(next_page_id).unwrap());
            self.idx = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::NamedTempFile;

    use crate::{
        buffer::bpm::BufferPoolManager,
        catalog::{column::Column, schema::Schema, types::SqlType},
        storage::disk::disk_manager::DiskManager,
        types::value::Value,
    };

    use super::*;

    fn setup_bpm(pool_size: usize) -> BufferPoolManager {
        let file = NamedTempFile::new().unwrap();
        let disk_manager = DiskManager::new(file.path().to_path_buf()).unwrap();
        BufferPoolManager::new(pool_size, disk_manager)
    }

    #[test]
    fn insert_and_get_tuples_across_multiple_pages() {
        let bpm = setup_bpm(3);
        let table_heap = TableHeap::new(&bpm).unwrap();

        let tuple_count = 200;
        let tuple_size = 64;
        let mut inserted = Vec::new();

        for idx in 0..tuple_count {
            let meta = TupleMeta::new(idx, idx % 3 == 0);
            let tuple = Tuple::from_bytes(vec![idx as u8; tuple_size]);
            let rid = table_heap.insert_tuple(&meta, &tuple).unwrap();

            inserted.push((rid, meta, tuple));
        }

        assert!(
            inserted
                .windows(2)
                .any(|window| window[0].0.page_id != window[1].0.page_id)
        );

        for (rid, expected_meta, expected_tuple) in inserted {
            let (actual_meta, actual_tuple) = table_heap.get_tuple(rid).unwrap();

            assert_eq!(actual_meta, expected_meta);
            assert_eq!(actual_tuple.data(), expected_tuple.data());
        }
    }

    #[test]
    fn insert_and_get_schema_backed_tuple_values() {
        let bpm = setup_bpm(3);
        let table_heap = TableHeap::new(&bpm).unwrap();
        let schema = Schema::new(&[
            Column::new_static("id".to_string(), SqlType::Integer),
            Column::new_variable("name".to_string(), SqlType::Varchar, 32),
            Column::new_static("score".to_string(), SqlType::Decimal),
            Column::new_variable("nickname".to_string(), SqlType::Varchar, 32),
        ]);
        let values = vec![
            Value::Integer(42),
            Value::Varchar("alice".to_string()),
            Value::Decimal(98.5),
            Value::Null(SqlType::Varchar),
        ];
        let meta = TupleMeta::new(7, false);
        let tuple = Tuple::from_values(&values, &schema);

        let rid = table_heap.insert_tuple(&meta, &tuple).unwrap();
        let (actual_meta, actual_tuple) = table_heap.get_tuple(rid).unwrap();

        assert_eq!(actual_meta, meta);
        assert_eq!(actual_tuple.get_values(&schema), values);
    }
}
