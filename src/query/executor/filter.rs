use crate::{
    catalog::schema::Schema,
    query::{
        executor::{engine::ExecutorRow, error::ExecutionError, executor::Executor},
        planner::plan::FilterPlan,
    },
};

pub struct FilterExecutor<'plan> {
    plan: &'plan FilterPlan,
    output_schema: &'plan Schema,
    child: Box<dyn Executor + 'plan>,
}

impl<'plan> FilterExecutor<'plan> {
    pub fn new(
        plan: &'plan FilterPlan,
        output_schema: &'plan Schema,
        child: Box<dyn Executor + 'plan>,
    ) -> Self {
        Self {
            plan,
            output_schema,
            child,
        }
    }
}

impl Executor for FilterExecutor<'_> {
    fn init(&mut self) -> Result<(), ExecutionError> {
        todo!("init filter executor")
    }

    fn next(&mut self, _batch_size: usize) -> Result<Vec<ExecutorRow>, ExecutionError> {
        todo!("next filter executor")
    }

    fn output_schema(&self) -> &Schema {
        self.output_schema
    }
}
