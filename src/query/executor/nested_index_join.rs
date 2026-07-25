use crate::{
    catalog::schema::Schema,
    query::{
        executor::{
            ExecutionError, Executor, ExecutorContext, ExecutorRow,
            expression::evaluate_expression_on_tuple, join::build_join_tuple,
        },
        planner::plan::NestedIndexJoinPlan,
        table_ref::JoinType,
    },
    storage::{rid::Rid, table::tuple::Tuple},
};

pub struct NestedIndexJoinExecutor<'ctx, 'catalog, 'bpm, 'plan> {
    exec_ctx: &'ctx ExecutorContext<'catalog, 'bpm>,
    plan: &'plan NestedIndexJoinPlan,
    output_schema: &'plan Schema,
    // executor for outer table
    outer_child: Box<dyn Executor + 'plan>,
    buffered_outer_tuples: Vec<ExecutorRow>,
    outer_tuple_offset: usize,
    outer_tuple_matched: bool,
    matching_inner_rids: Option<std::vec::IntoIter<Rid>>,
}

impl<'ctx, 'catalog, 'bpm, 'plan> NestedIndexJoinExecutor<'ctx, 'catalog, 'bpm, 'plan> {
    pub fn new(
        exec_ctx: &'ctx ExecutorContext<'catalog, 'bpm>,
        plan: &'plan NestedIndexJoinPlan,
        output_schema: &'plan Schema,
        outer_child: Box<dyn Executor + 'plan>,
    ) -> Self {
        Self {
            exec_ctx,
            plan,
            output_schema,
            outer_child,
            buffered_outer_tuples: Vec::new(),
            outer_tuple_offset: 0,
            outer_tuple_matched: false,
            matching_inner_rids: None,
        }
    }
}

impl Executor for NestedIndexJoinExecutor<'_, '_, '_, '_> {
    fn init(&mut self) -> Result<(), ExecutionError> {
        self.buffered_outer_tuples = Vec::new();
        self.outer_tuple_offset = 0;
        self.outer_tuple_matched = false;
        self.matching_inner_rids = None;
        self.outer_child.init()
    }

    fn next(&mut self, batch_size: usize) -> Result<Vec<ExecutorRow>, ExecutionError> {
        let mut out = Vec::with_capacity(batch_size);
        let index_info = self
            .exec_ctx
            .catalog
            .get_idx_by_oid(self.plan.inner_table_index_oid)?;
        let tbl_info = self
            .exec_ctx
            .catalog
            .get_tbl_by_oid(self.plan.inner_table_oid)?;

        let metadata = index_info.index.metadata();

        loop {
            if self.outer_tuple_offset >= self.buffered_outer_tuples.len() {
                let batch = self.outer_child.next(batch_size)?;
                if batch.is_empty() {
                    return Ok(out);
                }

                // Start a fresh batch of outer tuples.
                self.buffered_outer_tuples = batch;
                self.outer_tuple_offset = 0;
                self.outer_tuple_matched = false;
                self.matching_inner_rids = None;
            }

            let curr_outer_tuple_row = self.buffered_outer_tuples[self.outer_tuple_offset].clone();

            if self.matching_inner_rids.is_none() {
                let curr_outer_tuple = Tuple::from_values(
                    &curr_outer_tuple_row.values,
                    self.outer_child.output_schema(),
                );
                let outer_key_val = Tuple::from_values(
                    &self
                        .plan
                        .index_expressions
                        .iter()
                        .map(|expr| {
                            evaluate_expression_on_tuple(
                                expr,
                                &curr_outer_tuple,
                                self.outer_child.output_schema(),
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    &metadata.key_schema,
                );

                let matching_inner_rids = if outer_key_val.all_values_not_null(&metadata.key_schema)
                {
                    index_info.index.scan_key(&outer_key_val)?
                } else {
                    Vec::new()
                };
                self.matching_inner_rids = Some(matching_inner_rids.into_iter());
            }

            let mut matching_inner_rids = self.matching_inner_rids.take().unwrap();
            while let Some(rid) = matching_inner_rids.next() {
                let (tuple_meta, tuple) = tbl_info.table_heap.get_tuple(rid)?;

                if tuple_meta.is_deleted() {
                    continue;
                }

                self.outer_tuple_matched = true;
                let inner_row = ExecutorRow {
                    rid: Some(rid),
                    values: tuple.get_values(&self.plan.inner_table_schema),
                };

                out.push(build_join_tuple(
                    self.output_schema,
                    &curr_outer_tuple_row,
                    Some(&inner_row),
                ));

                if out.len() >= batch_size {
                    self.matching_inner_rids = Some(matching_inner_rids);
                    return Ok(out);
                }
            }

            if !self.outer_tuple_matched && self.plan.join_type == JoinType::Left {
                out.push(build_join_tuple(
                    self.output_schema,
                    &curr_outer_tuple_row,
                    None,
                ));
            }

            self.outer_tuple_offset += 1;
            self.outer_tuple_matched = false;

            if out.len() >= batch_size {
                return Ok(out);
            }
        }
    }

    fn output_schema(&self) -> &Schema {
        self.output_schema
    }
}
