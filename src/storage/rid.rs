use crate::common::types::PageId;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Rid {
    // 8 bytes total
    pub page_id: PageId,
    pub slot_id: u16,
    pub _reserved: u16,
}

impl Rid {
    pub fn new(page_id: PageId, slot_id: usize) -> Self {
        assert!(slot_id <= u16::MAX as usize);

        Self {
            page_id,
            slot_id: slot_id as u16,
            _reserved: 0,
        }
    }
}
