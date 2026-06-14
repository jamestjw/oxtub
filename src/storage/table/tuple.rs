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
// +----------+----------+----------+-------------------+
// | a value  | b offset | c value  | b payload          |
// | 4 bytes  | 4 bytes  | 4 bytes  | len + string bytes |
// +----------+----------+----------+-------------------+
// ^          ^          ^          ^
// 0          4          8          12
pub struct Tuple {
    data: Vec<u8>,
}

impl Tuple {
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

#[repr(transparent)]
pub struct VarOffset(pub u32);
