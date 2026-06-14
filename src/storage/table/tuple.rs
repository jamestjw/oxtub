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
}
