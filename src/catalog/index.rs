use crate::{catalog::schema::Schema, storage::index::index::Index};

pub type IndexId = u32;

pub struct IndexInfo {
    // size of the key in bytes
    key_size: usize,
    index_oid: IndexId,
    index: Box<dyn Index>,
}

impl IndexInfo {
    pub fn new(key_size: usize, index_oid: IndexId, index: Box<dyn Index>) -> Self {
        Self {
            key_size,
            index_oid,
            index,
        }
    }
}
