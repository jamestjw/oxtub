use crate::{
    catalog::schema::Schema,
    query::{
        executor::{
            engine::ExecutorRow, error::ExecutionError, executor::Executor,
            expression::filter_keep_row,
        },
        planner::plan::FilterPlan,
    },
};

pub struct FilterExecutor<'plan> {
    plan: &'plan FilterPlan,
    output_schema: &'plan Schema,
    child: Box<dyn Executor + 'plan>,
    buffered_rows: std::vec::IntoIter<ExecutorRow>,
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
            buffered_rows: Vec::new().into_iter(),
        }
    }
}

impl Executor for FilterExecutor<'_> {
    fn init(&mut self) -> Result<(), ExecutionError> {
        self.child.init()
    }

    fn next(&mut self, batch_size: usize) -> Result<Vec<ExecutorRow>, ExecutionError> {
        let mut out = Vec::with_capacity(batch_size);
        let predicate = &self.plan.predicate;

        loop {
            while let Some(row) = self.buffered_rows.next() {
                if filter_keep_row(predicate, &row)? {
                    out.push(row);

                    if out.len() == batch_size {
                        return Ok(out);
                    }
                }
            }

            let batch = self.child.next(batch_size)?;
            if batch.is_empty() {
                return Ok(out);
            }

            self.buffered_rows = batch.into_iter();
            while let Some(row) = self.buffered_rows.next() {
                if filter_keep_row(predicate, &row)? {
                    out.push(row);

                    if out.len() == batch_size {
                        return Ok(out);
                    }
                }
            }
        }
    }

    fn output_schema(&self) -> &Schema {
        self.output_schema
    }
}
