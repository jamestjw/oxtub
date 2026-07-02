use crate::{
    catalog::schema::Schema,
    query::{
        executor::{
            engine::ExecutorRow, error::ExecutionError, executor::Executor,
            expression::evaluate_expression,
        },
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
        self.child.init()
    }

    fn next(&mut self, batch_size: usize) -> Result<Vec<ExecutorRow>, ExecutionError> {
        let batch = self.child.next(batch_size)?;
        let mut res = Vec::with_capacity(batch.len());

        for row in batch {
            let row_values = self
                .plan
                .expressions
                .iter()
                .map(|expr| evaluate_expression(expr, &row))
                .collect::<Result<Vec<_>, _>>()?;

            res.push(ExecutorRow {
                rid: None,
                values: row_values,
            });
        }

        Ok(res)
    }

    fn output_schema(&self) -> &Schema {
        self.output_schema
    }
}
