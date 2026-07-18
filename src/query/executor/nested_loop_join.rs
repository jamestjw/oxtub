use crate::{
    catalog::schema::Schema,
    query::{
        executor::{ExecutionError, Executor, ExecutorContext, ExecutorRow},
        planner::plan::NestedLoopJoinPlan,
    },
};

pub struct NestedLoopJoinExecutor<'ctx, 'catalog, 'bpm, 'plan> {
    exec_ctx: &'ctx ExecutorContext<'catalog, 'bpm>,
    plan: &'plan NestedLoopJoinPlan,
    output_schema: &'plan Schema,
    left_child: Box<dyn Executor + 'plan>,
    right_child: Box<dyn Executor + 'plan>,
}

impl<'ctx, 'catalog, 'bpm, 'plan> NestedLoopJoinExecutor<'ctx, 'catalog, 'bpm, 'plan> {
    pub fn new(
        exec_ctx: &'ctx ExecutorContext<'catalog, 'bpm>,
        plan: &'plan NestedLoopJoinPlan,
        output_schema: &'plan Schema,
        left_child: Box<dyn Executor + 'plan>,
        right_child: Box<dyn Executor + 'plan>,
    ) -> Self {
        Self {
            exec_ctx,
            plan,
            output_schema,
            left_child,
            right_child,
        }
    }
}

impl Executor for NestedLoopJoinExecutor<'_, '_, '_, '_> {
    fn init(&mut self) -> Result<(), ExecutionError> {
        self.left_child.init()
    }

    fn next(&mut self, batch_size: usize) -> Result<Vec<ExecutorRow>, ExecutionError> {
        todo!()
    }

    fn output_schema(&self) -> &Schema {
        todo!()
    }
}
