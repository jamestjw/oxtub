use crate::{catalog::schema::Schema, storage::table::table_heap::TableHeap};

pub type TableId = u32;

pub struct TableInfo<'a> {
    pub(crate) schema: Schema,
    name: String,
    pub(crate) table_heap: TableHeap<'a>,
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

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn table_oid(&self) -> TableId {
        self.table_oid
    }

    pub fn schema(&self) -> Schema {
        self.schema.clone()
    }
}
