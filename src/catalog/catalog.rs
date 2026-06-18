use std::collections::HashMap;

use crate::{
    buffer::bpm::BufferPoolManager,
    catalog::{
        error::CatalogError,
        index::{IndexId, IndexInfo},
        schema::Schema,
        table::{TableId, TableInfo},
    },
    storage::table::table_heap::TableHeap,
};

pub struct Catalog<'a> {
    bpm: &'a BufferPoolManager,
    tables: HashMap<TableId, TableInfo<'a>>,
    indexes: HashMap<IndexId, IndexInfo>,
    table_names: HashMap<String, IndexId>,
    // table_name -> index_name -> index_oid
    table_index_names: HashMap<String, HashMap<String, IndexId>>,

    // TODO: evaluate if we should make this atomic, since I think
    // we should allow concurrent accesses to this struct?
    next_table_oid: TableId,
    next_index_oid: IndexId,
}

impl<'a> Catalog<'a> {
    pub fn new(bpm: &'a BufferPoolManager) -> Self {
        Self {
            bpm,
            tables: HashMap::new(),
            indexes: HashMap::new(),
            table_names: HashMap::new(),
            table_index_names: HashMap::new(),
            next_table_oid: 0,
            next_index_oid: 0,
        }
    }

    pub fn create_tbl(
        &mut self,
        name: String,
        schema: Schema,
    ) -> Result<&TableInfo<'_>, CatalogError> {
        if self.table_names.contains_key(&name) {
            return Err(CatalogError::DuplicateTableName(name));
        }

        let table_heap = TableHeap::new(self.bpm)?;
        let table_oid = self.next_table_oid;
        self.next_table_oid += 1;
        let table_info = TableInfo::new(schema, name.clone(), table_heap, table_oid);
        self.tables.insert(table_oid, table_info);
        self.table_names.insert(name.clone(), table_oid);
        self.table_index_names.insert(name, HashMap::new());

        Ok(self.tables.get(&table_oid).unwrap())
    }
}
