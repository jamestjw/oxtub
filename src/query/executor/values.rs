use crate::{
    catalog::schema::Schema,
    query::{
        executor::{
            engine::ExecutorRow, error::ExecutionError, executor::Executor,
            expression::evaluate_expression,
        },
        planner::plan::ValuesPlan,
    },
};

pub struct ValuesExecutor<'plan> {
    plan: &'plan ValuesPlan,
    output_schema: &'plan Schema,
    next_row_idx: usize,
}

impl<'plan> ValuesExecutor<'plan> {
    pub fn new(plan: &'plan ValuesPlan, output_schema: &'plan Schema) -> Self {
        Self {
            plan,
            output_schema,
            next_row_idx: 0,
        }
    }
}

impl Executor for ValuesExecutor<'_> {
    fn init(&mut self) -> Result<(), ExecutionError> {
        self.next_row_idx = 0;
        Ok(())
    }

    fn next(&mut self, batch_size: usize) -> Result<Vec<ExecutorRow>, ExecutionError> {
        let end = (self.next_row_idx + batch_size).min(self.plan.rows.len());
        let mut rows = Vec::with_capacity(end.saturating_sub(self.next_row_idx));
        let empty_row = ExecutorRow {
            rid: None,
            values: vec![],
        };

        for row in &self.plan.rows[self.next_row_idx..end] {
            let values = row
                .iter()
                // these values should not be bound to anything, so it should
                // be fine to use an empty row as the basis for the exprs
                .map(|expr| evaluate_expression(expr, &empty_row))
                .collect::<Result<Vec<_>, _>>()?;

            rows.push(ExecutorRow { rid: None, values });
        }

        self.next_row_idx = end;
        Ok(rows)
    }

    fn output_schema(&self) -> &Schema {
        self.output_schema
    }
}
