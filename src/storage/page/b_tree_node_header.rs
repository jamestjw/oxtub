pub const PAGE_TYPE_INVALID: u8 = 0;
pub const PAGE_TYPE_LEAF: u8 = 1;
pub const PAGE_TYPE_INTERNAL: u8 = 2;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BTreeNodeHeader {
    // 8 byte header, we do explicit padding for clarity
    page_type: u8,
    _padding: u8,
    pub(crate) current_size: u16,
    pub(crate) max_size: u16,
    _reserved: u16,
}

impl BTreeNodeHeader {
    pub fn init(&mut self, page_type: u8, max_size: usize) {
        assert!(max_size <= u16::MAX as usize);

        self.page_type = page_type;
        self._padding = 0;
        self.current_size = 0;
        self.max_size = max_size as u16;
        self._reserved = 0;
    }

    pub fn is_leaf(&self) -> bool {
        self.page_type == PAGE_TYPE_LEAF
    }

    pub fn set_size(&mut self, size: usize) {
        self.current_size = size as u16;
    }
}
