use crate::{
    catalog::{schema::Schema, types::SqlType},
    storage::{index::generic_key::GenericKey, table::tuple::Tuple},
    types::value::Value,
};

pub fn encode_one_col_key<const N: usize>(tuple: &Tuple, schema: &Schema) -> GenericKey<N> {
    let mut data = [0; N];
    encode_value(&tuple.get_value(schema, 0), &mut data);
    GenericKey::from_bytes(data)
}

pub fn encode_two_col_key<const N: usize>(tuple: &Tuple, schema: &Schema) -> GenericKey<N> {
    let mut data = [0; N];
    let first_len = schema.columns()[0].sql_type().inline_size();

    encode_value(&tuple.get_value(schema, 0), &mut data[..first_len]);
    encode_value(&tuple.get_value(schema, 1), &mut data[first_len..]);

    GenericKey::from_bytes(data)
}

pub fn encoded_key_size(sql_types: &[SqlType]) -> Option<usize> {
    sql_types
        .iter()
        .map(|sql_type| match sql_type {
            SqlType::Boolean => Some(size_of::<u8>()),
            SqlType::SmallInt => Some(size_of::<i16>()),
            SqlType::Integer => Some(size_of::<i32>()),
            SqlType::BigInt => Some(size_of::<i64>()),
            SqlType::Decimal => Some(size_of::<f64>()),
            SqlType::Varchar => None,
        })
        .sum()
}

fn encode_value(value: &Value, data: &mut [u8]) {
    match value {
        Value::Boolean(v) => data[0] = *v as u8,
        Value::SmallInt(v) => data.copy_from_slice(&(v ^ i16::MIN).to_be_bytes()),
        Value::Integer(v) => data.copy_from_slice(&(v ^ i32::MIN).to_be_bytes()),
        Value::BigInt(v) => data.copy_from_slice(&(v ^ i64::MIN).to_be_bytes()),
        Value::Decimal(v) => data.copy_from_slice(&encode_f64(*v).to_be_bytes()),
        Value::Varchar(_) | Value::Null(_) => {
            unreachable!("key schema was validated at index creation")
        }
    }
}

fn encode_f64(value: f64) -> u64 {
    let bits = value.to_bits();
    if bits & (1 << 63) == 0 {
        bits ^ (1 << 63)
    } else {
        !bits
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        catalog::{column::Column, schema::Schema, types::SqlType},
        storage::index::comparator::KeyComparator,
    };

    use super::*;

    #[test]
    fn two_i32_key_encoding_preserves_tuple_order() {
        let schema = Schema::new(&[
            Column::new_static("a".to_string(), SqlType::Integer),
            Column::new_static("b".to_string(), SqlType::Integer),
        ]);
        let tuples = [
            Tuple::from_values(&[Value::Integer(-1), Value::Integer(10)], &schema),
            Tuple::from_values(&[Value::Integer(0), Value::Integer(-1)], &schema),
            Tuple::from_values(&[Value::Integer(0), Value::Integer(1)], &schema),
            Tuple::from_values(&[Value::Integer(1), Value::Integer(-10)], &schema),
        ];
        let keys = tuples
            .iter()
            .map(|tuple| encode_two_col_key::<8>(tuple, &schema))
            .collect::<Vec<_>>();
        let comparator = crate::storage::index::generic_key::GenericKeyComparator;

        for pair in keys.windows(2) {
            assert!(comparator.compare(&pair[0], &pair[1]).is_lt());
        }
    }

    #[test]
    fn encoded_key_size_supports_fixed_width_types_only() {
        assert_eq!(
            encoded_key_size(&[SqlType::Boolean, SqlType::SmallInt]),
            Some(3)
        );
        assert_eq!(
            encoded_key_size(&[SqlType::Integer, SqlType::BigInt]),
            Some(12)
        );
        assert_eq!(
            encoded_key_size(&[SqlType::Decimal, SqlType::Decimal]),
            Some(16)
        );
        assert_eq!(
            encoded_key_size(&[SqlType::Integer, SqlType::Varchar]),
            None
        );
    }
}
