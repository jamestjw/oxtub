use crate::{
    buffer::bpm::BufferPoolManager,
    catalog::schema::Schema,
    storage::{
        index::{
            b_tree::BTree,
            error::{BTreeError, IndexError},
            generic_key::{GenericKey, GenericKeyComparator},
            index::{Index, IndexMetadata},
        },
        rid::Rid,
        table::tuple::Tuple,
    },
};

type KeyEncoder<const N: usize> = fn(tuple: &Tuple, schema: &Schema) -> GenericKey<N>;

pub struct BTreeIndex<'a, const N: usize> {
    metadata: IndexMetadata,
    tree: BTree<'a, GenericKey<N>, GenericKeyComparator, 0>,
    key_encoder: KeyEncoder<N>,
}

impl<'a, const N: usize> BTreeIndex<'a, N> {
    pub fn new(
        bpm: &'a BufferPoolManager,
        metadata: IndexMetadata,
        key_encoder: KeyEncoder<N>,
    ) -> Self {
        let header_page_id = bpm.new_page();
        Self {
            metadata,
            key_encoder,
            tree: BTree::new(bpm, header_page_id, &GenericKeyComparator),
        }
    }
}

impl<'a, const N: usize> Index for BTreeIndex<'a, N> {
    fn metadata(&self) -> &IndexMetadata {
        &self.metadata
    }

    fn insert_entry(&self, key: &Tuple, rid: Rid) -> Result<(), IndexError> {
        let key = (self.key_encoder)(key, &self.metadata.key_schema);
        match self.tree.insert(key, rid) {
            Ok(_) => Ok(()),
            Err(BTreeError::Duplicate) => Err(IndexError::DuplicateKey),
            Err(BTreeError::BufferPool(e)) => Err(IndexError::BufferPool(e)),
            e => panic!("unexpected error {e:?}"),
        }
    }

    fn scan_key(&self, key: &Tuple) -> Result<Vec<Rid>, IndexError> {
        let key = (self.key_encoder)(key, &self.metadata.key_schema);
        match self.tree.get_values(&key) {
            Ok(r) => Ok(r),
            Err(BTreeError::BufferPool(e)) => Err(IndexError::BufferPool(e)),
            e => panic!("unexpected error {e:?}"),
        }
    }

    fn delete_entry(
        &self,
        key: &crate::storage::table::tuple::Tuple,
        rid: crate::storage::rid::Rid,
    ) -> Result<(), super::error::IndexError> {
        let key = (self.key_encoder)(key, &self.metadata.key_schema);
        match self.tree.remove(key, rid) {
            Ok(r) => Ok(r),
            Err(BTreeError::NotFound) => Err(IndexError::KeyNotFound),
            Err(BTreeError::EmptyTree) => Err(IndexError::KeyNotFound),
            Err(BTreeError::BufferPool(e)) => Err(IndexError::BufferPool(e)),
            e => panic!("unexpected error {e:?}"),
        }
    }
}
