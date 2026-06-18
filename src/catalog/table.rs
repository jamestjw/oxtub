use crate::{catalog::schema::Schema, storage::table::table_heap::TableHeap};

pub type TableId = u32;

pub struct TableInfo<'a> {
    schema: Schema,
    name: String,
    table_heap: TableHeap<'a>,
    table_oid: TableId,
}

impl<'a> TableInfo<'a> {
    pub fn new(
        schema: Schema,
        name: String,
        table_heap: TableHeap<'a>,
        table_oid: TableId,
    ) -> Self {
        Self {
            schema,
            name,
            table_heap,
            table_oid,
        }
    }
}
