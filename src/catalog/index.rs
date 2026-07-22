use crate::{catalog::schema::Schema, storage::index::index::Index};

pub type IndexId = u32;

pub struct IndexInfo<'a> {
    // size of the key in bytes
    key_size: usize,
    index_oid: IndexId,
    pub(crate) index: Box<dyn Index + 'a>,
}

impl<'a> IndexInfo<'a> {
    pub fn new(key_size: usize, index_oid: IndexId, index: Box<dyn Index + 'a>) -> Self {
        Self {
            key_size,
            index_oid,
            index,
        }
    }

    pub fn oid(&self) -> IndexId {
        self.index_oid
    }
}
