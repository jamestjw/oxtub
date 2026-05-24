pub const PAGE_TYPE_INVALID: u8 = 0;
pub const PAGE_TYPE_LEAF: u8 = 1;
pub const PAGE_TYPE_INTERNAL: u8 = 2;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BTreePageHeader {
    // 8 byte header, we do explicit padding for clarity
    page_type: u8,
    _padding: u8,
    current_size: u16,
    pub(crate) max_size: u16,
    _reserved: u16,
}

impl BTreePageHeader {
    pub fn is_leaf(&self) -> bool {
        self.page_type == PAGE_TYPE_LEAF
    }
}
