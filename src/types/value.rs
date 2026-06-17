use crate::{catalog::types::SqlType, storage::table::tuple::VarSize};

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

    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null(_))
    }

    // includes the u32 (VarSize) that we use to store the length
    // of a variable length data type
    pub fn variable_storage_size(&self) -> usize {
        match self {
            Value::Varchar(str) => str.len() + size_of::<VarSize>(),
            Value::Null(_) => 0,
            _ => panic!("should not use this for non variable storage"),
        }
    }

    pub fn serialize_to(&self, data: &mut [u8]) {
        // inlined columns will be written as is, variable length columns will
        // write out the size in u32, then the rest of the data
        match self {
            Value::Boolean(b) => data.copy_from_slice(bytemuck::bytes_of(&(*b as u8))),
            Value::SmallInt(i) => data.copy_from_slice(bytemuck::bytes_of(i)),
            Value::Integer(i) => data.copy_from_slice(bytemuck::bytes_of(i)),
            Value::BigInt(i) => data.copy_from_slice(bytemuck::bytes_of(i)),
            Value::Decimal(f) => data.copy_from_slice(bytemuck::bytes_of(f)),
            Value::Varchar(s) => {
                data[..size_of::<VarSize>()]
                    .copy_from_slice(bytemuck::bytes_of(&(VarSize(s.len() as u32))));
                data[size_of::<VarSize>()..].copy_from_slice(s.as_bytes());
            }
            Value::Null(_) => panic!("should be tracking nulls with the null bitmap"),
        }
    }

    pub fn deserialize_from(data: &[u8], sql_type: SqlType) -> Self {
        match sql_type {
            SqlType::Boolean => Value::Boolean(data[0] != 0),
            SqlType::SmallInt => {
                Value::SmallInt(bytemuck::pod_read_unaligned(&data[..size_of::<i16>()]))
            }
            SqlType::Integer => {
                Value::Integer(bytemuck::pod_read_unaligned(&data[..size_of::<i32>()]))
            }
            SqlType::BigInt => {
                Value::BigInt(bytemuck::pod_read_unaligned(&data[..size_of::<i64>()]))
            }
            SqlType::Decimal => {
                Value::Decimal(bytemuck::pod_read_unaligned(&data[..size_of::<f64>()]))
            }
            SqlType::Varchar => {
                let var_size: VarSize = bytemuck::pod_read_unaligned(&data[..size_of::<VarSize>()]);
                let start = size_of::<VarSize>();
                let end = start + usize::from(var_size);
                let s = String::from_utf8(data[start..end].to_vec()).unwrap();
                Value::Varchar(s)
            }
        }
    }
}
