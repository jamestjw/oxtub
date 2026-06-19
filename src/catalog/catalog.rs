use std::collections::HashMap;

use crate::{
    buffer::bpm::BufferPoolManager,
    catalog::{
        error::CatalogError,
        index::{IndexId, IndexInfo},
        schema::Schema,
        table::{TableId, TableInfo},
    },
    storage::{
        index::{factory::build_index, index::IndexMetadata},
        table::table_heap::TableHeap,
    },
};

pub struct Catalog<'a> {
    bpm: &'a BufferPoolManager,
    tables: HashMap<TableId, TableInfo<'a>>,
    indexes: HashMap<IndexId, IndexInfo<'a>>,
    table_names: HashMap<String, TableId>,
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
            return Err(CatalogError::DuplicateTable(name));
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

    pub fn get_tbl_by_name(&self, name: &str) -> Result<&TableInfo<'_>, CatalogError> {
        match self.table_names.get(name) {
            Some(table_oid) => match self.tables.get(table_oid) {
                Some(info) => Ok(info),
                None => panic!("table oid invalid?"),
            },
            None => Err(CatalogError::TableNotFound(name.into())),
        }
    }

    pub fn get_tbl_by_oid(&self, oid: TableId) -> Result<&TableInfo<'_>, CatalogError> {
        match self.tables.get(&oid) {
            Some(info) => Ok(info),
            None => Err(CatalogError::TableNotFound(oid.into())),
        }
    }

    pub fn create_index(
        &mut self,
        index_name: String,
        table_name: String,
        key_schema: Schema,
        key_attrs: Vec<usize>,
        key_size: usize,
        is_pk: bool,
    ) -> Result<&IndexInfo<'_>, CatalogError> {
        if let None = self.table_names.get(&table_name) {
            return Err(CatalogError::TableNotFound(table_name.as_str().into()));
        }

        let table_schema = match self.table_names.get(&table_name) {
            None => return Err(CatalogError::TableNotFound(table_name.as_str().into())),
            Some(oid) => match self.tables.get(oid) {
                Some(table_info) => &table_info.schema,
                None => panic!("table info not found"),
            },
        };

        {
            let table_index_names = self
                .table_index_names
                .get(&table_name)
                .expect("table does not exist?");

            if table_index_names.contains_key(&index_name) {
                return Err(CatalogError::DuplicateIndex(index_name));
            }
        }

        let table_meta = self.get_tbl_by_name(&table_name)?;

        let metadata = IndexMetadata {
            name: index_name.clone(),
            table_name: table_name.clone(),
            key_schema: key_schema.clone(),
            key_attrs: key_attrs.clone(),
            is_pk,
        };

        let mut index = build_index(
            self.bpm,
            table_schema,
            &key_schema,
            &key_attrs,
            key_size,
            metadata,
        )?;

        for (rid, tuple_meta, tuple) in table_meta.table_heap.iter() {
            let key = tuple.key_from_tuple(table_schema, &key_schema, &key_attrs);

            if tuple_meta.is_deleted() {
                continue;
            }

            index.insert_entry(&key, rid)?;
        }

        let index_oid = self.next_index_oid;
        let index_info = IndexInfo::new(key_size, index_oid, index);
        self.next_index_oid += 1;

        self.indexes.insert(index_oid, index_info);
        self.table_index_names
            .get_mut(&table_name)
            .unwrap()
            .insert(index_name, index_oid);

        Ok(&self.indexes.get(&index_oid).unwrap())
    }
}
