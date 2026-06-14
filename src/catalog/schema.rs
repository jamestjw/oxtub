use std::mem::size_of;

use crate::{catalog::column::Column, storage::table::tuple::VarOffset};

pub struct Schema {
    // number of bytes occupied by the fix-length part of the tuple
    // refer to src/storage/table/tuple.rs for tuple layout
    fixed_length_size: usize,
    columns: Vec<Column>,
    // indices of columns that are not inlined
    uninlined_columns: Vec<usize>,
}

impl Schema {
    pub fn new(columns: &[Column]) -> Self {
        let mut curr_offset = 0;
        let mut processed_columns = Vec::with_capacity(columns.len());
        let mut uninlined_columns = vec![];

        for (i, col) in columns.iter().enumerate() {
            let mut col = col.clone();

            if !col.is_inlined() {
                uninlined_columns.push(i);
            }
            col.value_offset = curr_offset;

            if col.is_inlined() {
                curr_offset += col.size();
            } else {
                curr_offset += size_of::<VarOffset>();
            }

            processed_columns.push(col);
        }

        Self {
            fixed_length_size: curr_offset,
            columns: processed_columns,
            uninlined_columns,
        }
    }

    pub fn is_entirely_inlined(&self) -> bool {
        self.uninlined_columns.is_empty()
    }
}
