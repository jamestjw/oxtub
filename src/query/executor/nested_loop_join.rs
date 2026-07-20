use crate::{
    catalog::schema::Schema,
    query::{
        executor::{
            ExecutionError, Executor, ExecutorContext, ExecutorRow, expression::filter_join_row,
        },
        planner::plan::NestedLoopJoinPlan,
        table_ref::JoinType,
    },
    types::value::Value,
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
            None => {
                let right_null_values = output_schema.columns()[values.len()..]
                    .iter()
                    .map(|col| Value::Null(col.sql_type()))
                    .collect::<Vec<_>>();
                values.extend(right_null_values);
            }
        }
        ExecutorRow { rid: None, values }
    }

    pub fn keep_row(
        &self,
        left: &ExecutorRow,
        right: &ExecutorRow,
    ) -> Result<bool, ExecutionError> {
        match (&self.plan.predicate, self.plan.join_type) {
            (_, JoinType::Cross) => Ok(true),
            (None, _) => Ok(true),
            (Some(predicate), JoinType::Inner | JoinType::Left) => {
                filter_join_row(predicate, left, right)
            }
        }
    }
}

impl Executor for NestedLoopJoinExecutor<'_, '_, '_, '_> {
    fn init(&mut self) -> Result<(), ExecutionError> {
        self.left_child.init()
    }

    fn next(&mut self, batch_size: usize) -> Result<Vec<ExecutorRow>, ExecutionError> {
        let mut out = Vec::with_capacity(batch_size);

        loop {
            if self.outer_tuple_offset >= self.buffered_outer_tuples.len() {
                let batch = self.left_child.next(batch_size)?;
                if batch.is_empty() {
                    return Ok(out);
                }

                // Running fresh batch of outer tuples, reset right child so
                // we start all over again.
                self.buffered_outer_tuples = batch;
                self.outer_tuple_offset = 0;
                self.outer_tuple_matched = false;
                self.right_child.init()?;
            }

            let curr_outer_tuple = &self.buffered_outer_tuples[self.outer_tuple_offset];

            if self.buffered_inner_tuples.len() == 0 {
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
                    self.right_child.init()?;

                    if out.len() >= batch_size {
                        return Ok(out);
                    }

                    // go back to outer loop
                    continue;
                }

                self.buffered_inner_tuples = batch.into_iter();
            }

            while let Some(inner_tuple) = self.buffered_inner_tuples.next() {
                if self.keep_row(curr_outer_tuple, &inner_tuple)? {
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

#[cfg(test)]
mod tests {
    use crate::{
        catalog::{column::Column, manager::Catalog, schema::Schema, types::SqlType},
        query::{
            executor::{ExecutorContext, engine::ExecutorRow},
            planner::{
                expression::{
                    ColumnValueExpression, ComparisonExpression, ComparisonType, ExpressionType,
                    PlannedExpression, PlannedExpressionKind,
                },
                plan::{NestedLoopJoinPlan, PlanNode, PlanNodeKind, ValuesPlan},
            },
            table_ref::JoinType,
        },
        testing::setup_bpm,
        types::value::Value,
    };

    use super::*;

    struct MockExecutor {
        schema: Schema,
        rows: Vec<ExecutorRow>,
        next_row_idx: usize,
    }

    impl MockExecutor {
        fn new(schema: Schema, values: &[i32]) -> Self {
            Self {
                schema,
                rows: values
                    .iter()
                    .map(|value| ExecutorRow {
                        rid: None,
                        values: vec![Value::Integer(*value)],
                    })
                    .collect(),
                next_row_idx: 0,
            }
        }
    }

    impl Executor for MockExecutor {
        fn init(&mut self) -> Result<(), ExecutionError> {
            self.next_row_idx = 0;
            Ok(())
        }

        fn next(&mut self, batch_size: usize) -> Result<Vec<ExecutorRow>, ExecutionError> {
            let end = (self.next_row_idx + batch_size).min(self.rows.len());
            let rows = self.rows[self.next_row_idx..end].to_vec();
            self.next_row_idx = end;
            Ok(rows)
        }

        fn output_schema(&self) -> &Schema {
            &self.schema
        }
    }

    fn int_schema(name: &str) -> Schema {
        Schema::new(&[Column::new_static(name.to_string(), SqlType::Integer)])
    }

    fn dummy_values_plan(output_schema: Schema) -> PlanNode {
        PlanNode {
            output_schema,
            kind: PlanNodeKind::Values(ValuesPlan { rows: vec![] }),
        }
    }

    fn int_col(tuple_idx: usize) -> PlannedExpression {
        PlannedExpression {
            return_type: ExpressionType {
                sql_type: SqlType::Integer,
                varchar_size: None,
            },
            kind: PlannedExpressionKind::ColumnValue(ColumnValueExpression {
                tuple_idx,
                col_idx: 0,
            }),
        }
    }

    fn eq_join_predicate() -> PlannedExpression {
        PlannedExpression {
            return_type: ExpressionType::new_bool(),
            kind: PlannedExpressionKind::Comparison(ComparisonExpression {
                left: Box::new(int_col(0)),
                comparison_type: ComparisonType::Eq,
                right: Box::new(int_col(1)),
            }),
        }
    }

    fn join_plan(
        join_type: JoinType,
        left_schema: Schema,
        right_schema: Schema,
    ) -> NestedLoopJoinPlan {
        NestedLoopJoinPlan {
            left: Box::new(dummy_values_plan(left_schema)),
            right: Box::new(dummy_values_plan(right_schema)),
            join_type,
            predicate: Some(eq_join_predicate()),
        }
    }

    fn collect_join_batches(
        executor: &mut NestedLoopJoinExecutor<'_, '_, '_, '_>,
        batch_size: usize,
    ) -> Vec<Vec<ExecutorRow>> {
        executor.init().unwrap();

        let mut batches = vec![];
        loop {
            let batch = executor.next(batch_size).unwrap();
            if batch.is_empty() {
                break;
            }
            batches.push(batch);
        }

        batches
    }

    fn joined_values(left: i32, right: Value) -> ExecutorRow {
        ExecutorRow {
            rid: None,
            values: vec![Value::Integer(left), right],
        }
    }

    #[test]
    fn inner_join_resumes_buffered_inner_rows_across_batches() {
        let bpm = setup_bpm(3);
        let catalog = Catalog::new(&bpm);
        let exec_ctx = ExecutorContext::new(&catalog);
        let left_schema = int_schema("left_id");
        let right_schema = int_schema("right_id");
        let output_schema = Schema::new(&[
            Column::new_static("left_id".to_string(), SqlType::Integer),
            Column::new_static("right_id".to_string(), SqlType::Integer),
        ]);
        let plan = join_plan(JoinType::Inner, left_schema.clone(), right_schema.clone());
        let mut executor = NestedLoopJoinExecutor::new(
            &exec_ctx,
            &plan,
            &output_schema,
            Box::new(MockExecutor::new(left_schema, &[1, 2])),
            Box::new(MockExecutor::new(right_schema, &[1, 1, 2])),
        );

        let batches = collect_join_batches(&mut executor, 2);

        assert_eq!(
            batches,
            vec![
                vec![
                    joined_values(1, Value::Integer(1)),
                    joined_values(1, Value::Integer(1)),
                ],
                vec![joined_values(2, Value::Integer(2))],
            ]
        );
    }

    #[test]
    fn left_join_emits_unmatched_outer_rows_across_batches() {
        let bpm = setup_bpm(3);
        let catalog = Catalog::new(&bpm);
        let exec_ctx = ExecutorContext::new(&catalog);
        let left_schema = int_schema("left_id");
        let right_schema = int_schema("right_id");
        let output_schema = Schema::new(&[
            Column::new_static("left_id".to_string(), SqlType::Integer),
            Column::new_static("right_id".to_string(), SqlType::Integer),
        ]);
        let plan = join_plan(JoinType::Left, left_schema.clone(), right_schema.clone());
        let mut executor = NestedLoopJoinExecutor::new(
            &exec_ctx,
            &plan,
            &output_schema,
            Box::new(MockExecutor::new(left_schema, &[1, 2, 3])),
            Box::new(MockExecutor::new(right_schema, &[1, 3])),
        );

        let batches = collect_join_batches(&mut executor, 1);

        assert_eq!(
            batches,
            vec![
                vec![joined_values(1, Value::Integer(1))],
                vec![joined_values(2, Value::Null(SqlType::Integer))],
                vec![joined_values(3, Value::Integer(3))],
            ]
        );
    }
}
