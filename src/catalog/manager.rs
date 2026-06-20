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
        if !self.table_names.contains_key(&table_name) {
            return Err(CatalogError::TableNotFound(table_name.as_str().into()));
        }

        let table_schema = match self.table_names.get(&table_name) {
            None => return Err(CatalogError::TableNotFound(table_name.as_str().into())),
            Some(oid) => match self.tables.get(oid) {
                Some(table_info) => &table_info.schema,
                None => panic!("table info not found"),
            },
        };

        if self
            .table_index_names
            .get(&table_name)
            .unwrap()
            .contains_key(&index_name)
        {
            return Err(CatalogError::DuplicateIndex(index_name));
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

        Ok(self.indexes.get(&index_oid).unwrap())
    }

    pub fn get_idx_for_tbl_name(
        &self,
        index_name: &str,
        tbl_name: &str,
    ) -> Result<&IndexInfo<'a>, CatalogError> {
        match self.table_index_names.get(tbl_name) {
            None => Err(CatalogError::TableNotFound(tbl_name.into())),
            Some(index_map) => match index_map.get(index_name) {
                None => Err(CatalogError::IndexNotFound(index_name.into())),
                Some(index_oid) => Ok(self.indexes.get(index_oid).expect("index not found")),
            },
        }
    }

    pub fn get_idx_for_tbl_oid(
        &self,
        index_name: &str,
        tbl_oid: TableId,
    ) -> Result<&IndexInfo<'a>, CatalogError> {
        match self.tables.get(&tbl_oid) {
            None => Err(CatalogError::TableNotFound(tbl_oid.into())),
            Some(tbl_info) => self.get_idx_for_tbl_name(index_name, tbl_info.name()),
        }
    }

    pub fn get_idx_by_oid(&self, index_oid: IndexId) -> Result<&IndexInfo<'a>, CatalogError> {
        match self.indexes.get(&index_oid) {
            None => Err(CatalogError::IndexNotFound(index_oid.into())),
            Some(idx_info) => Ok(idx_info),
        }
    }

    pub fn get_table_indexes(&self, table_name: &str) -> Result<Vec<&IndexInfo<'a>>, CatalogError> {
        match self.table_index_names.get(table_name) {
            None => Err(CatalogError::TableNotFound(table_name.into())),
            Some(index_map) => Ok(index_map
                .values()
                .map(|idx_id| self.get_idx_by_oid(*idx_id).unwrap())
                .collect::<Vec<_>>()),
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::NamedTempFile;

    use crate::{
        buffer::bpm::BufferPoolManager,
        catalog::{column::Column, types::SqlType},
        storage::disk::disk_manager::DiskManager,
    };

    use super::*;

    fn setup_bpm(pool_size: usize) -> BufferPoolManager {
        let file = NamedTempFile::new().unwrap();
        let disk_manager = DiskManager::new(file.path().to_path_buf()).unwrap();
        BufferPoolManager::new(pool_size, disk_manager)
    }

    fn int_schema() -> Schema {
        Schema::new(&[Column::new_static("id".to_string(), SqlType::Integer)])
    }

    fn create_users_table(catalog: &mut Catalog<'_>) {
        catalog
            .create_tbl("users".to_string(), int_schema())
            .unwrap();
    }

    fn create_users_id_index(catalog: &mut Catalog<'_>) {
        catalog
            .create_index(
                "idx_users_id".to_string(),
                "users".to_string(),
                int_schema(),
                vec![0],
                size_of::<i32>(),
                false,
            )
            .unwrap();
    }

    fn unwrap_catalog_err<T>(result: Result<T, CatalogError>) -> CatalogError {
        match result {
            Ok(_) => panic!("expected catalog error"),
            Err(err) => err,
        }
    }

    #[test]
    fn create_and_get_table() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);

        let table = catalog
            .create_tbl("users".to_string(), int_schema())
            .unwrap();

        assert_eq!(table.name(), "users");
        assert_eq!(catalog.get_tbl_by_name("users").unwrap().name(), "users");
        assert_eq!(catalog.get_tbl_by_oid(0).unwrap().name(), "users");
    }

    #[test]
    fn duplicate_table_returns_error() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);

        create_users_table(&mut catalog);
        let err = unwrap_catalog_err(catalog.create_tbl("users".to_string(), int_schema()));

        assert!(matches!(err, CatalogError::DuplicateTable(name) if name == "users"));
    }

    #[test]
    fn missing_table_returns_error() {
        let bpm = setup_bpm(3);
        let catalog = Catalog::new(&bpm);

        let by_name = unwrap_catalog_err(catalog.get_tbl_by_name("missing"));
        let by_oid = unwrap_catalog_err(catalog.get_tbl_by_oid(99));

        assert!(matches!(by_name, CatalogError::TableNotFound(_)));
        assert!(matches!(by_oid, CatalogError::TableNotFound(_)));
    }

    #[test]
    fn create_and_get_index() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);

        create_users_table(&mut catalog);
        create_users_id_index(&mut catalog);

        assert!(
            catalog
                .get_idx_for_tbl_name("idx_users_id", "users")
                .is_ok()
        );
        assert!(catalog.get_idx_for_tbl_oid("idx_users_id", 0).is_ok());
        assert!(catalog.get_idx_by_oid(0).is_ok());
        assert_eq!(catalog.get_table_indexes("users").unwrap().len(), 1);
    }

    #[test]
    fn duplicate_index_returns_error() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);

        create_users_table(&mut catalog);
        create_users_id_index(&mut catalog);
        let err = unwrap_catalog_err(catalog.create_index(
            "idx_users_id".to_string(),
            "users".to_string(),
            int_schema(),
            vec![0],
            size_of::<i32>(),
            false,
        ));

        assert!(matches!(err, CatalogError::DuplicateIndex(name) if name == "idx_users_id"));
    }

    #[test]
    fn missing_index_returns_error() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);

        create_users_table(&mut catalog);
        let by_name = unwrap_catalog_err(catalog.get_idx_for_tbl_name("missing_idx", "users"));
        let by_oid = unwrap_catalog_err(catalog.get_idx_by_oid(99));

        assert!(matches!(by_name, CatalogError::IndexNotFound(_)));
        assert!(matches!(by_oid, CatalogError::IndexNotFound(_)));
    }
}
