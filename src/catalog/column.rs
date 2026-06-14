#[derive(Debug, Clone, Copy)]
pub struct Column {
    // offset within the tuple for data corresponding to the column
    pub(crate) value_offset: usize,
}

impl Column {
    // size required to store the inlined part of this column
    pub fn size(&self) -> usize {
        todo!()
    }

    pub fn is_inlined(&self) -> bool {
        todo!()
    }
}
