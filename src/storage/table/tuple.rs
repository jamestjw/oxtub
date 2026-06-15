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
}

#[repr(transparent)]
pub struct VarOffset(pub u32);

#[repr(transparent)]
pub struct VarSize(pub u32);

const VAR_NULL_SIZE: VarSize = VarSize(u32::MAX);

// Fixed-size columns like INTEGER, BIGINT, BOOLEAN, etc. are stored directly inside the tuple's
// fixed-size region. Variable-size columns are stored as a 4-byte offset in the fixed-size
// region, and the actual payload later in the tuple.
//
// Example schema:
// (a INTEGER, b VARCHAR, c INTEGER)
//
// Tuple layout:
//
// Tuple data_
// +----------+--------------+----------+------------------------+
// | a value  | b VarOffset  | c value  | b payload              |
// | 4 bytes  | 4 bytes      | 4 bytes  | VarSize + string bytes |
// +----------+--------------+----------+------------------------+
// ^          ^              ^          ^
// 0          4              8          12
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
            .map(|idx| values[*idx].variable_storage_size() + size_of::<VarSize>())
            .sum::<usize>();
        let tuple_size = inlined_size + not_inlined_size;

        let mut data = vec![0; tuple_size];
        let mut variable_data_offset = inlined_size;

        for (val, col) in values.iter().zip(schema.columns()) {
            let inline_start = col.value_offset;
            let inline_end = inline_start + col.inline_size();

            if col.is_inlined() {
                val.serialize_to(&mut data[inline_start..inline_end]);
            } else {
                data[inline_start..inline_end].copy_from_slice(&variable_data_offset.to_le_bytes());
                let variable_data_size = size_of::<VarSize>() + val.variable_storage_size();
                val.serialize_to(
                    &mut data[variable_data_offset..(variable_data_offset + variable_data_size)],
                );
                variable_data_offset += variable_data_size;
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
}
