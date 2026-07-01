use crate::{
    catalog::schema::Schema,
    query::{
        executor::{engine::ExecutorRow, error::ExecutionError, executor::Executor},
        planner::plan::ValuesPlan,
    },
};

pub struct ValuesExecutor<'plan> {
    plan: &'plan ValuesPlan,
    output_schema: &'plan Schema,
}

impl<'plan> ValuesExecutor<'plan> {
    pub fn new(plan: &'plan ValuesPlan, output_schema: &'plan Schema) -> Self {
        Self {
            plan,
            output_schema,
        }
    }
}

impl Executor for ValuesExecutor<'_> {
    fn init(&mut self) -> Result<(), ExecutionError> {
        todo!("init values executor")
    }

    fn next(&mut self, _batch_size: usize) -> Result<Vec<ExecutorRow>, ExecutionError> {
        todo!("next values executor")
    }

    fn output_schema(&self) -> &Schema {
        self.output_schema
    }
}
