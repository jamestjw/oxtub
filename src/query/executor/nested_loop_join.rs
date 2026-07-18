use crate::{
    catalog::schema::Schema,
    query::{
        executor::{
            ExecutionError, Executor, ExecutorContext, ExecutorRow, expression::filter_join_row,
        },
        planner::plan::NestedLoopJoinPlan,
        table_ref::JoinType,
    },
};

pub struct NestedLoopJoinExecutor<'ctx, 'catalog, 'bpm, 'plan> {
    exec_ctx: &'ctx ExecutorContext<'catalog, 'bpm>,
    plan: &'plan NestedLoopJoinPlan,
    output_schema: &'plan Schema,
    left_child: Box<dyn Executor + 'plan>,
    right_child: Box<dyn Executor + 'plan>,
    buffered_outer_tuples: Vec<ExecutorRow>,
    outer_tuple_offset: usize,
    outer_tuple_matched: bool,
    buffered_inner_tuples: std::vec::IntoIter<ExecutorRow>,
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
            buffered_inner_tuples: Vec::new().into_iter(),
            buffered_outer_tuples: Vec::new(),
            outer_tuple_offset: 0,
            outer_tuple_matched: false,
        }
    }

    fn build_join_tuple(
        output_schema: &'plan Schema,
        left_tuple: &ExecutorRow,
        right_tuple: Option<&ExecutorRow>,
    ) -> ExecutorRow {
        let mut values = Vec::with_capacity(output_schema.num_columns());
        values.extend_from_slice(&left_tuple.values);

        match right_tuple {
            Some(right_tuple) => values.extend_from_slice(&right_tuple.values),
            None => todo!(),
        }
        ExecutorRow { rid: None, values }
    }
}

impl Executor for NestedLoopJoinExecutor<'_, '_, '_, '_> {
    fn init(&mut self) -> Result<(), ExecutionError> {
        self.left_child.init()
    }

    fn next(&mut self, batch_size: usize) -> Result<Vec<ExecutorRow>, ExecutionError> {
        let mut out = Vec::with_capacity(batch_size);
        let predicate = &self.plan.predicate;

        loop {
            if self.buffered_outer_tuples.len() == 0 {
                let batch = self.left_child.next(batch_size)?;
                if batch.is_empty() {
                    return Ok(out);
                }

                // Running fresh batch of outer tuples, reset right child so
                // we start all over again.
                self.buffered_outer_tuples = batch;
                self.outer_tuple_offset = 0;
                self.outer_tuple_matched = false;
                self.right_child.init();
            }

            let curr_outer_tuple = &self.buffered_outer_tuples[self.outer_tuple_offset];

            if (self.buffered_inner_tuples.len() == 0) {
                let batch = self.right_child.next(batch_size)?;
                if batch.is_empty() {
                    // Inner loop complete, go back to the outer loop to get a new outer tuple
                    if self.plan.join_type == JoinType::Left && !self.outer_tuple_matched {
                        let joined_tuple =
                            Self::build_join_tuple(self.output_schema, curr_outer_tuple, None);
                        out.push(joined_tuple);
                    }

                    self.outer_tuple_offset += 1;
                    self.outer_tuple_matched = false;
                    self.right_child.init();

                    if out.len() >= batch_size {
                        return Ok(out);
                    }

                    // go back to outer loop
                    continue;
                }

                self.buffered_inner_tuples = batch.into_iter();
            }

            while let Some(inner_tuple) = self.buffered_inner_tuples.next() {
                let keep_row = match predicate {
                    Some(predicate) => filter_join_row(predicate, &curr_outer_tuple, &inner_tuple)?,
                    None => true,
                };

                if keep_row {
                    self.outer_tuple_matched = true;
                    out.push(Self::build_join_tuple(
                        self.output_schema,
                        curr_outer_tuple,
                        Some(&inner_tuple),
                    ));

                    if out.len() >= batch_size {
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
