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
