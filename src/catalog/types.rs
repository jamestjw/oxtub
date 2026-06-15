use std::mem::size_of;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlType {
    Boolean,
    SmallInt,
    Integer,
    BigInt,
    Decimal,
    Varchar,
}

impl SqlType {
    pub fn is_inlined(self) -> bool {
        !matches!(self, Self::Varchar)
    }

    // The size occupied in the inlined part of the tuple
    pub fn inline_size(self) -> usize {
        match self {
            Self::Boolean => size_of::<u8>(),
            Self::SmallInt => size_of::<i16>(),
            Self::Integer => size_of::<i32>(),
            Self::BigInt => size_of::<i64>(),
            Self::Decimal => size_of::<f64>(),
            Self::Varchar => panic!("should not use this fn for size of variable types"),
        }
    }
}
