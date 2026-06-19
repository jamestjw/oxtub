use crate::{catalog::column::Column, storage::table::tuple::NullBitmap};

#[derive(Clone)]
pub struct Schema {
    // number of bytes occupied by the fix-length part of the tuple
    // refer to src/storage/table/tuple.rs for tuple layout
    inlined_storage_size: usize,
    columns: Vec<Column>,
    // indices of columns that are not inlined
    uninlined_columns: Vec<usize>,
}

impl Schema {
    pub fn new(columns: &[Column]) -> Self {
        let mut curr_offset = NullBitmap::num_bytes(columns.len());
        let mut processed_columns = Vec::with_capacity(columns.len());
        let mut uninlined_columns = vec![];

        for (i, col) in columns.iter().enumerate() {
            let mut col = col.clone();

            if !col.is_inlined() {
                uninlined_columns.push(i);
            }
            col.value_offset = curr_offset;

            curr_offset += col.inline_size();

            processed_columns.push(col);
        }

        Self {
            inlined_storage_size: curr_offset,
            columns: processed_columns,
            uninlined_columns,
        }
    }

    pub fn is_entirely_inlined(&self) -> bool {
        self.uninlined_columns.is_empty()
    }

    pub fn num_columns(&self) -> usize {
        self.columns.len()
    }

    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    pub fn inlined_storage_size(&self) -> usize {
        self.inlined_storage_size
    }

    pub fn uninlined_column_idxs(&self) -> &[usize] {
        &self.uninlined_columns
    }
}
