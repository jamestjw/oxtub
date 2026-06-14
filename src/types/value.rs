use crate::catalog::types::SqlType;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Boolean(bool),
    SmallInt(i16),
    Integer(i32),
    BigInt(i64),
    Decimal(f64),
    Varchar(String),
    Null(SqlType),
}

impl Value {
    pub fn sql_type(&self) -> SqlType {
        match self {
            Self::Boolean(_) => SqlType::Boolean,
            Self::SmallInt(_) => SqlType::SmallInt,
            Self::Integer(_) => SqlType::Integer,
            Self::BigInt(_) => SqlType::BigInt,
            Self::Decimal(_) => SqlType::Decimal,
            Self::Varchar(_) => SqlType::Varchar,
            Self::Null(sql_type) => *sql_type,
        }
    }

    pub fn variable_storage_size(&self) -> usize {
        match self {
            Value::Varchar(str) => str.len(),
            Value::Null(_) => 0,
            _ => panic!("should not use this for non variable storage"),
        }
    }
}
