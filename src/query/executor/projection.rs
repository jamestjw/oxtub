use crate::{
    catalog::schema::Schema,
    query::{
        executor::{engine::ExecutorRow, error::ExecutionError, executor::Executor},
        planner::plan::ProjectionPlan,
    },
};

pub struct ProjectionExecutor<'plan> {
    plan: &'plan ProjectionPlan,
    output_schema: &'plan Schema,
    child: Box<dyn Executor + 'plan>,
}

impl<'plan> ProjectionExecutor<'plan> {
    pub fn new(
        plan: &'plan ProjectionPlan,
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

impl Executor for ProjectionExecutor<'_> {
    fn init(&mut self) -> Result<(), ExecutionError> {
        todo!("init projection executor")
    }

    fn next(&mut self, _batch_size: usize) -> Result<Vec<ExecutorRow>, ExecutionError> {
        todo!("next projection executor")
    }

    fn output_schema(&self) -> &Schema {
        self.output_schema
    }
}
