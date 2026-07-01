use crate::{
    catalog::schema::Schema,
    query::{
        executor::{
            engine::ExecutorRow,
            error::ExecutionError,
            executor::{Executor, ExecutorContext},
        },
        planner::plan::SeqScanPlan,
    },
};

pub struct SeqScanExecutor<'ctx, 'catalog, 'bpm, 'plan> {
    exec_ctx: &'ctx ExecutorContext<'catalog, 'bpm>,
    plan: &'plan SeqScanPlan,
    output_schema: &'plan Schema,
}

impl<'ctx, 'catalog, 'bpm, 'plan> SeqScanExecutor<'ctx, 'catalog, 'bpm, 'plan> {
    pub fn new(
        exec_ctx: &'ctx ExecutorContext<'catalog, 'bpm>,
        plan: &'plan SeqScanPlan,
        output_schema: &'plan Schema,
    ) -> Self {
        Self {
            exec_ctx,
            plan,
            output_schema,
        }
    }
}

impl Executor for SeqScanExecutor<'_, '_, '_, '_> {
    fn init(&mut self) -> Result<(), ExecutionError> {
        todo!("init seq scan executor")
    }

    fn next(&mut self, _batch_size: usize) -> Result<Vec<ExecutorRow>, ExecutionError> {
        todo!("next seq scan executor")
    }

    fn output_schema(&self) -> &Schema {
        self.output_schema
    }
}
