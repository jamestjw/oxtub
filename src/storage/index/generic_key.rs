use std::cmp::Ordering;

use crate::storage::index::comparator::KeyComparator;

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Debug, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GenericKey<const N: usize> {
    data: [u8; N],
}

impl<const N: usize> GenericKey<N> {
    pub fn from_bytes(data: [u8; N]) -> Self {
        Self { data }
    }
}

pub struct GenericKeyComparator;

impl<const N: usize> KeyComparator<GenericKey<N>> for GenericKeyComparator {
    fn compare(&self, a: &GenericKey<N>, b: &GenericKey<N>) -> Ordering {
        a.data.cmp(&b.data)
    }
}
