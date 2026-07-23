use std::error::Error;

use crate::{
    catalog::schema::Schema,
    storage::{index::error::IndexError, rid::Rid, table::tuple::Tuple},
};

pub struct IndexMetadata {
    pub name: String,
    pub table_name: String,
    pub key_schema: Schema,
    pub key_attrs: Vec<usize>,
    pub is_pk: bool,
}

pub trait Index: Send {
    fn metadata(&self) -> &IndexMetadata;
    fn insert_entry(&self, key: &Tuple, rid: Rid) -> Result<(), IndexError>;
    fn delete_entry(&self, key: &Tuple, rid: Rid) -> Result<(), IndexError>;
    fn scan_key(&self, key: &Tuple) -> Result<Vec<Rid>, IndexError>;
}
