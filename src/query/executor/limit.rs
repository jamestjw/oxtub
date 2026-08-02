use crate::{
    catalog::schema::Schema,
    query::{
        executor::{
            engine::ExecutorRow, error::ExecutionError, executor::Executor,
            expression::evaluate_expression,
        },
        planner::plan::LimitPlan,
    },
    types::value::Value,
};

pub struct LimitExecutor<'plan> {
    plan: &'plan LimitPlan,
    output_schema: &'plan Schema,
    child: Box<dyn Executor + 'plan>,
    remaining: Option<usize>,
}

impl<'plan> LimitExecutor<'plan> {
    pub fn new(
        plan: &'plan LimitPlan,
        output_schema: &'plan Schema,
        child: Box<dyn Executor + 'plan>,
    ) -> Self {
        Self {
            plan,
            output_schema,
            child,
            remaining: Some(0),
        }
    }
}

impl Executor for LimitExecutor<'_> {
    fn init(&mut self) -> Result<(), ExecutionError> {
        self.child.init()?;

        let empty_row = ExecutorRow {
            rid: None,
            values: vec![],
        };
        self.remaining = match evaluate_expression(&self.plan.limit, &empty_row)? {
            Value::Null(_) => None,
            Value::SmallInt(value) if value >= 0 => Some(value as usize),
            Value::Integer(value) if value >= 0 => Some(value as usize),
            Value::BigInt(value) if value >= 0 => Some(
                value
                    .try_into()
                    .map_err(|_| ExecutionError::NumericOutOfRange)?,
            ),
            Value::SmallInt(_) | Value::Integer(_) | Value::BigInt(_) => {
                return Err(ExecutionError::NegativeLimit);
            }
            value => return Err(ExecutionError::ExpectedInteger(value)),
        };

        Ok(())
    }

    fn next(&mut self, batch_size: usize) -> Result<Vec<ExecutorRow>, ExecutionError> {
        if self.remaining == Some(0) {
            return Ok(vec![]);
        }

        let child_batch_size = self
            .remaining
            .map_or(batch_size, |remaining| batch_size.min(remaining));
        let batch = self.child.next(child_batch_size)?;
        if let Some(remaining) = &mut self.remaining {
            *remaining -= batch.len();
        }
        Ok(batch)
    }

    fn output_schema(&self) -> &Schema {
        self.output_schema
    }
}
