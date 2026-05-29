#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Rid {
    // 8 bytes total
    pub page_id: u32,
    pub slot_id: u16,
    pub _reserved: u16,
}

impl Rid {
    pub fn new(page_id: usize, slot_id: usize) -> Self {
        assert!(page_id <= u32::MAX as usize);
        assert!(slot_id <= u16::MAX as usize);

        Self {
            page_id: page_id as u32,
            slot_id: slot_id as u16,
            _reserved: 0,
        }
    }
}
