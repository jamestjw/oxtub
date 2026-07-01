use crate::{
    catalog::schema::Schema,
    query::{
        executor::{
            engine::ExecutorRow,
            error::ExecutionError,
            executor::{Executor, ExecutorContext},
        },
        planner::plan::DeletePlan,
    },
};

pub struct DeleteExecutor<'ctx, 'catalog, 'bpm, 'plan> {
    exec_ctx: &'ctx ExecutorContext<'catalog, 'bpm>,
    plan: &'plan DeletePlan,
    output_schema: &'plan Schema,
    child: Box<dyn Executor + 'plan>,
}

impl<'ctx, 'catalog, 'bpm, 'plan> DeleteExecutor<'ctx, 'catalog, 'bpm, 'plan> {
    pub fn new(
        exec_ctx: &'ctx ExecutorContext<'catalog, 'bpm>,
        plan: &'plan DeletePlan,
        output_schema: &'plan Schema,
        child: Box<dyn Executor + 'plan>,
    ) -> Self {
        Self {
            exec_ctx,
            plan,
            output_schema,
            child,
        }
    }
}

impl Executor for DeleteExecutor<'_, '_, '_, '_> {
    fn init(&mut self) -> Result<(), ExecutionError> {
        todo!("init delete executor")
    }

    fn next(&mut self, _batch_size: usize) -> Result<Vec<ExecutorRow>, ExecutionError> {
        todo!("next delete executor")
    }

    fn output_schema(&self) -> &Schema {
        self.output_schema
    }
}
