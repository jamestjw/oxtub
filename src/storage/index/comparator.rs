use std::cmp::Ordering;

pub trait KeyComparator<K> {
    fn compare(&self, a: &K, b: &K) -> Ordering;
}
