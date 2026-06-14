use std::cmp::Ordering;

use crate::storage::index::comparator::KeyComparator;

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Debug, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GenericKey<const N: usize> {
    data: [u8; N],
}

impl GenericKey<4> {
    pub fn from_i32(value: i32) -> Self {
        Self {
            data: (value ^ i32::MIN).to_be_bytes(),
        }
    }

    pub fn to_i32(self) -> i32 {
        i32::from_be_bytes(self.data) ^ i32::MIN
    }
}

impl GenericKey<8> {
    pub fn from_i64(value: i64) -> Self {
        Self {
            data: (value ^ i64::MIN).to_be_bytes(),
        }
    }

    pub fn from_i32_i32(value: (i32, i32)) -> Self {
        let (a, b) = value;
        let mut data = [0; 8];
        data[..4].copy_from_slice(&((a ^ i32::MIN).to_be_bytes()));
        data[4..].copy_from_slice(&((b ^ i32::MIN).to_be_bytes()));
        Self { data }
    }
}

pub struct GenericKeyComparator;

impl<const N: usize> KeyComparator<GenericKey<N>> for GenericKeyComparator {
    fn compare(&self, a: &GenericKey<N>, b: &GenericKey<N>) -> Ordering {
        a.data.cmp(&b.data)
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::index::comparator::KeyComparator;

    use super::*;

    #[test]
    fn i32_encoding_preserves_order() {
        let comparator = GenericKeyComparator;
        let values = [i32::MIN, -2, -1, 0, 1, 2, i32::MAX];
        let keys: Vec<_> = values
            .iter()
            .map(|&value| GenericKey::<4>::from_i32(value))
            .collect();

        for pair in keys.windows(2) {
            assert!(comparator.compare(&pair[0], &pair[1]).is_lt());
        }
    }

    #[test]
    fn i64_encoding_preserves_order() {
        let comparator = GenericKeyComparator;

        let values = [i64::MIN, -2, -1, 0, 1, 2, i64::MAX];
        let keys: Vec<_> = values
            .iter()
            .map(|&value| GenericKey::<8>::from_i64(value))
            .collect();

        for pair in keys.windows(2) {
            assert!(comparator.compare(&pair[0], &pair[1]).is_lt());
        }
    }

    #[test]
    fn i32_i32_encoding_preserves_tuple_order() {
        let comparator = GenericKeyComparator;

        let values = [
            (-1, -1),
            (-1, 0),
            (-1, 1),
            (0, 0),
            (0, 1),
            (1, 0),
            (2, 3),
            (2, 4),
            (i32::MAX, i32::MIN),
        ];

        let keys: Vec<_> = values
            .iter()
            .map(|&value| GenericKey::<8>::from_i32_i32(value))
            .collect();

        for pair in keys.windows(2) {
            assert!(comparator.compare(&pair[0], &pair[1]).is_lt());
        }
    }
}
