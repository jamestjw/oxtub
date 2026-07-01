use crate::{
    catalog::schema::Schema,
    query::{
        executor::{
            engine::ExecutorRow,
            error::ExecutionError,
            executor::{Executor, ExecutorContext},
        },
        planner::plan::InsertPlan,
    },
};

pub struct InsertExecutor<'ctx, 'catalog, 'bpm, 'plan> {
    exec_ctx: &'ctx ExecutorContext<'catalog, 'bpm>,
    plan: &'plan InsertPlan,
    output_schema: &'plan Schema,
    child: Box<dyn Executor + 'plan>,
}

impl<'ctx, 'catalog, 'bpm, 'plan> InsertExecutor<'ctx, 'catalog, 'bpm, 'plan> {
    pub fn new(
        exec_ctx: &'ctx ExecutorContext<'catalog, 'bpm>,
        plan: &'plan InsertPlan,
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

impl Executor for InsertExecutor<'_, '_, '_, '_> {
    fn init(&mut self) -> Result<(), ExecutionError> {
        todo!("init insert executor")
    }

    fn next(&mut self, _batch_size: usize) -> Result<Vec<ExecutorRow>, ExecutionError> {
        todo!("next insert executor")
    }

    fn output_schema(&self) -> &Schema {
        self.output_schema
    }
}
