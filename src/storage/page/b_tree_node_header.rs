use crate::buffer::page::PageBytes;

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

    pub fn from_data(data: &PageBytes) -> &Self {
        bytemuck::from_bytes(&data[..size_of::<Self>()])
    }

    pub fn is_leaf(&self) -> bool {
        self.page_type == PAGE_TYPE_LEAF
    }

    pub fn set_size(&mut self, size: usize) {
        self.current_size = size as u16;
    }

    pub fn curr_size(&self) -> usize {
        self.current_size as usize
    }

    pub fn is_insert_safe(&self) -> bool {
        if self.is_leaf() {
            self.current_size + 1 < self.max_size
        } else {
            self.current_size < self.max_size
        }
    }

    pub fn min_size(&self) -> usize {
        let max_size = self.max_size as usize;

        if self.is_leaf() {
            max_size / 2
        } else {
            max_size.div_ceil(2)
        }
    }
}
