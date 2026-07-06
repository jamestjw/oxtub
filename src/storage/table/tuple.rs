use crate::{catalog::schema::Schema, types::value::Value};

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TupleMeta {
    txn_id: u64,
    is_deleted: u8,
    _padding: [u8; 7],
    // 16 bytes header
}

impl TupleMeta {
    pub fn new(txn_id: usize, is_deleted: bool) -> Self {
        Self {
            txn_id: txn_id as u64,
            is_deleted: if is_deleted { 1 } else { 0 },
            _padding: [0; 7],
        }
    }

    pub fn is_deleted(&self) -> bool {
        self.is_deleted != 0
    }

    pub fn delete(&self) -> Self {
        Self {
            is_deleted: 1,
            ..*self
        }
    }
}

pub(crate) struct NullBitmap<'a> {
    data: &'a [u8],
}

impl<'a> NullBitmap<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    pub fn num_bytes(num_columns: usize) -> usize {
        num_columns.div_ceil(8)
    }

    pub fn is_null(&self, col_idx: usize) -> bool {
        let byte_idx = col_idx / 8;
        let bit_idx = col_idx % 8;
        let mask = 1u8 << bit_idx;
        self.data[byte_idx] & mask != 0
    }
}

pub(crate) struct NullBitmapMut<'a> {
    data: &'a mut [u8],
}

impl<'a> NullBitmapMut<'a> {
    pub fn new(data: &'a mut [u8]) -> Self {
        Self { data }
    }

    pub fn set_null(&mut self, col_idx: usize) {
        let byte_idx = col_idx / 8;
        let bit_idx = col_idx % 8;
        let mask = 1u8 << bit_idx;
        self.data[byte_idx] |= mask;
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VarOffset(pub u32);

impl From<VarOffset> for usize {
    fn from(offset: VarOffset) -> Self {
        offset.0 as usize
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VarSize(pub u32);

impl From<VarSize> for usize {
    fn from(size: VarSize) -> Self {
        size.0 as usize
    }
}

// Tuple layout:
//
// Tuple data begins with a null bitmap. Each column gets one bit:
// bit = 1 means NULL, bit = 0 means non-NULL.
//
// Fixed-size columns like INTEGER, BIGINT, BOOLEAN, etc. are stored directly
// in the fixed-size region. Variable-size columns store a 4-byte VarOffset in
// the fixed-size region; the offset points to the variable-size payload later
// in the tuple.
//
// Variable-size payloads are encoded as:
//
// +---------+---------------+
// | VarSize | payload bytes |
// +---------+---------------+
//
// Null values are represented only by the null bitmap. For null variable-size
// values, no variable-size payload is written.
//
// Example schema:
// (a INTEGER, b VARCHAR, c INTEGER)
//
// Tuple data:
// +-------------+----------+-------------+----------+------------------------+
// | null bitmap | a value  | b VarOffset | c value  | b payload              |
// | 1 byte      | 4 bytes  | 4 bytes     | 4 bytes  | VarSize + string bytes |
// +-------------+----------+-------------+----------+------------------------+
// ^             ^          ^             ^          ^
// 0             1          5             9          13
pub struct Tuple {
    data: Vec<u8>,
}

impl Tuple {
    pub fn from_values(values: &[Value], schema: &Schema) -> Self {
        assert_eq!(values.len(), schema.num_columns());
        for (value, column) in values.iter().zip(schema.columns()) {
            assert_eq!(value.sql_type(), column.sql_type());
        }

        let inlined_size = schema.inlined_storage_size();

        // storage of the varchar + the u32 that represents the length
        let not_inlined_size = schema
            .uninlined_column_idxs()
            .iter()
            .map(|idx| values[*idx].variable_storage_size())
            .sum::<usize>();
        let tuple_size = inlined_size + not_inlined_size;

        let mut data = vec![0; tuple_size];
        let mut variable_data_offset = inlined_size;
        let null_bitmap_size = NullBitmap::num_bytes(values.len());

        for (col_idx, (val, col)) in values.iter().zip(schema.columns()).enumerate() {
            if val.is_null() {
                // When we know a column contains a null, we will never read the data
                // in the slot of the column, so we don't even bother writing it. The variable
                // length part at the end of the array is not even populated.
                let mut null_bitmap = NullBitmapMut::new(&mut data[..null_bitmap_size]);
                null_bitmap.set_null(col_idx);
            } else {
                let inline_start = col.value_offset;
                let inline_end = inline_start + col.inline_size();

                if col.is_inlined() {
                    val.serialize_to(&mut data[inline_start..inline_end]);
                } else {
                    let offset = VarOffset(variable_data_offset.try_into().unwrap());
                    data[inline_start..inline_end].copy_from_slice(bytemuck::bytes_of(&offset));

                    let variable_data_size = val.variable_storage_size();
                    val.serialize_to(
                        &mut data
                            [variable_data_offset..(variable_data_offset + variable_data_size)],
                    );
                    variable_data_offset += variable_data_size;
                }
            }
        }

        Self { data }
    }

    pub fn from_bytes(data: Vec<u8>) -> Self {
        Self { data }
    }

    pub fn size(&self) -> usize {
        self.data.len()
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn get_value(&self, schema: &Schema, idx: usize) -> Value {
        let column = &schema.columns()[idx];
        let sql_type = column.sql_type();

        if self.is_null(schema, idx) {
            Value::Null(sql_type)
        } else if column.is_inlined() {
            let value_range = column.value_offset..column.value_offset + sql_type.inline_size();
            Value::deserialize_from(&self.data[value_range], sql_type)
        } else {
            let offset_range = column.value_offset..column.value_offset + size_of::<VarOffset>();
            let var_offset: VarOffset = bytemuck::pod_read_unaligned(&self.data[offset_range]);

            Value::deserialize_from(&self.data[usize::from(var_offset)..], sql_type)
        }
    }

    pub fn get_values(&self, schema: &Schema) -> Vec<Value> {
        (0..schema.num_columns())
            .map(|idx| self.get_value(schema, idx))
            .collect()
    }

    fn is_null(&self, schema: &Schema, idx: usize) -> bool {
        let bitmap_size = NullBitmap::num_bytes(schema.num_columns());
        NullBitmap::new(&self.data[..bitmap_size]).is_null(idx)
    }

    pub fn key_from_tuple(
        &self,
        schema: &Schema,
        key_schema: &Schema,
        key_attrs: &[usize],
    ) -> Self {
        let mut values = Vec::with_capacity(key_attrs.len());

        for key_attr in key_attrs {
            values.push(self.get_value(schema, *key_attr));
        }

        Tuple::from_values(&values, key_schema)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{column::Column, types::SqlType};

    #[test]
    fn tuple_round_trips_non_null_values() {
        let schema = Schema::new(&[
            Column::new_static("bool".to_string(), SqlType::Boolean),
            Column::new_static("small".to_string(), SqlType::SmallInt),
            Column::new_static("int".to_string(), SqlType::Integer),
            Column::new_static("big".to_string(), SqlType::BigInt),
            Column::new_static("decimal".to_string(), SqlType::Decimal),
            Column::new_variable("varchar".to_string(), SqlType::Varchar, 32),
        ]);
        let values = [
            Value::Boolean(true),
            Value::SmallInt(-12),
            Value::Integer(12345),
            Value::BigInt(-9876543210),
            Value::Decimal(12.5),
            Value::Varchar("hello tuple".to_string()),
        ];

        let tuple = Tuple::from_values(&values, &schema);

        for (idx, value) in values.iter().enumerate() {
            assert_eq!(tuple.get_value(&schema, idx), *value);
        }
    }

    #[test]
    fn tuple_round_trips_null_values() {
        let schema = Schema::new(&[
            Column::new_static("int".to_string(), SqlType::Integer),
            Column::new_variable("nullable_varchar".to_string(), SqlType::Varchar, 32),
            Column::new_static("nullable_big".to_string(), SqlType::BigInt),
            Column::new_variable("varchar".to_string(), SqlType::Varchar, 32),
        ]);
        let values = [
            Value::Integer(7),
            Value::Null(SqlType::Varchar),
            Value::Null(SqlType::BigInt),
            Value::Varchar("not null".to_string()),
        ];

        let tuple = Tuple::from_values(&values, &schema);

        for (idx, value) in values.iter().enumerate() {
            assert_eq!(tuple.get_value(&schema, idx), *value);
        }
    }
}
