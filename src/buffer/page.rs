use crate::storage::disk::config::DEFAULT_PAGE_SIZE;

#[repr(align(8))]
pub struct PageData<const N: usize>(pub [u8; N]);

pub struct Page {
    page_id: Option<usize>,
    data: PageData<DEFAULT_PAGE_SIZE>,
}

impl Page {
    pub fn new() -> Self {
        Self {
            page_id: None,
            data: PageData([0; DEFAULT_PAGE_SIZE]),
        }
    }

    pub fn data(&self) -> &[u8; DEFAULT_PAGE_SIZE] {
        &self.data.0
    }
    pub fn data_mut(&mut self) -> &mut [u8; DEFAULT_PAGE_SIZE] {
        &mut self.data.0
    }
    pub fn page_id(&self) -> Option<usize> {
        self.page_id
    }
    pub fn set_page_id(&mut self, page_id: Option<usize>) {
        self.page_id = page_id;
    }
}
