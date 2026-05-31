use crate::{
    buffer::page::{INVALID_PAGE_ID, PageBytes},
    common::types::PageId,
};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BTreeRootPageData {
    root_page_id: PageId,
}

pub struct BTreeRootPage<'a> {
    data: &'a PageBytes,
}

pub struct BTreeRootPageMut<'a> {
    data: &'a mut PageBytes,
}

impl<'a> BTreeRootPage<'a> {
    pub fn from_data(data: &'a PageBytes) -> Self {
        Self { data }
    }

    fn root(&self) -> &BTreeRootPageData {
        bytemuck::from_bytes(&self.data[..size_of::<BTreeRootPageData>()])
    }

    pub fn root_page_id(&self) -> PageId {
        self.root().root_page_id
    }
}

impl<'a> BTreeRootPageMut<'a> {
    pub fn from_data(data: &'a mut PageBytes) -> Self {
        Self { data }
    }

    pub fn init(data: &'a mut PageBytes) -> Self {
        data.fill(0);

        let mut page = Self::from_data(data);
        page.root_mut().root_page_id = INVALID_PAGE_ID;
        page
    }

    fn root(&self) -> &BTreeRootPageData {
        bytemuck::from_bytes(&self.data[..size_of::<BTreeRootPageData>()])
    }

    fn root_mut(&mut self) -> &mut BTreeRootPageData {
        bytemuck::from_bytes_mut(&mut self.data[..size_of::<BTreeRootPageData>()])
    }

    pub fn root_page_id(&self) -> PageId {
        self.root().root_page_id
    }

    pub fn set_root_page_id(&mut self, root_page_id: PageId) {
        self.root_mut().root_page_id = root_page_id;
    }
}
