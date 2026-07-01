use crate::{
    catalog::{manager::Catalog, schema::Schema},
    query::executor::{engine::ExecutorRow, error::ExecutionError},
};

pub struct ExecutorContext<'catalog, 'bpm> {
    pub catalog: &'catalog Catalog<'bpm>,
}

impl<'catalog, 'bpm> ExecutorContext<'catalog, 'bpm> {
    pub fn new(catalog: &'catalog Catalog<'bpm>) -> Self {
        Self { catalog }
    }
}

pub trait Executor {
    fn init(&mut self) -> Result<(), ExecutionError>;

    fn next(&mut self, batch_size: usize) -> Result<Vec<ExecutorRow>, ExecutionError>;

    fn output_schema(&self) -> &Schema;
}
