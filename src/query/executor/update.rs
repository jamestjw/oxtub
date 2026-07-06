use crate::{
    catalog::schema::Schema,
    query::{
        executor::{
            engine::ExecutorRow,
            error::ExecutionError,
            executor::{Executor, ExecutorContext},
            expression::evaluate_expression_on_tuple,
        },
        planner::plan::UpdatePlan,
    },
    storage::table::tuple::{Tuple, TupleMeta},
    types::value::Value,
};

pub struct UpdateExecutor<'ctx, 'catalog, 'bpm, 'plan> {
    exec_ctx: &'ctx ExecutorContext<'catalog, 'bpm>,
    plan: &'plan UpdatePlan,
    output_schema: &'plan Schema,
    child: Box<dyn Executor + 'plan>,
    done: bool,
}

impl<'ctx, 'catalog, 'bpm, 'plan> UpdateExecutor<'ctx, 'catalog, 'bpm, 'plan> {
    pub fn new(
        exec_ctx: &'ctx ExecutorContext<'catalog, 'bpm>,
        plan: &'plan UpdatePlan,
        output_schema: &'plan Schema,
        child: Box<dyn Executor + 'plan>,
    ) -> Self {
        Self {
            exec_ctx,
            plan,
            output_schema,
            child,
            done: false,
        }
    }
}

impl Executor for UpdateExecutor<'_, '_, '_, '_> {
    fn init(&mut self) -> Result<(), ExecutionError> {
        self.done = false;
        self.child.init()
    }

    fn next(&mut self, batch_size: usize) -> Result<Vec<ExecutorRow>, ExecutionError> {
        if self.done {
            return Ok(vec![]);
        }

        let mut updated_count = 0;
        let table_info = self.exec_ctx.catalog.get_tbl_by_oid(self.plan.table_oid)?;
        let indexes = self
            .exec_ctx
            .catalog
            .get_table_indexes(&self.plan.table_name)?;

        loop {
            let batch = self.child.next(batch_size)?;
            if batch.is_empty() {
                break;
            }

            for ExecutorRow { rid, values: _ } in batch {
                let rid = rid.expect("update executor child produced row without rid");
                let (old_tuple_meta, old_tuple) = table_info.table_heap.get_tuple(rid)?;
                assert!(!old_tuple_meta.is_deleted(), "can't update deleted tuple");

                let new_tuple_values = self
                    .plan
                    .expressions
                    .iter()
                    .map(|expr| {
                        evaluate_expression_on_tuple(expr, &old_tuple, &self.plan.table_schema)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let new_tuple = Tuple::from_values(&new_tuple_values, &self.plan.table_schema);

                // mark original tuple as deleted
                table_info
                    .table_heap
                    .update_meta(rid, old_tuple_meta.delete())?;
                // TODO: make transaction ID something meaningful once we introduce transactions
                let new_rid = table_info
                    .table_heap
                    .insert_tuple(&TupleMeta::new(0, false), &new_tuple)?;

                for index_info in indexes.iter() {
                    let metadata = index_info.index.metadata();

                    // delete and recreate index entry
                    let old_key = old_tuple.key_from_tuple(
                        &self.plan.table_schema,
                        &metadata.key_schema,
                        &metadata.key_attrs,
                    );
                    let new_key = new_tuple.key_from_tuple(
                        &self.plan.table_schema,
                        &metadata.key_schema,
                        &metadata.key_attrs,
                    );
                    index_info.index.delete_entry(&old_key, rid)?;
                    index_info.index.insert_entry(&new_key, new_rid)?;
                }

                updated_count += 1;
            }
        }

        self.done = true;

        Ok(vec![ExecutorRow {
            rid: None,
            values: vec![Value::Integer(updated_count)],
        }])
    }

    fn output_schema(&self) -> &Schema {
        self.output_schema
    }
}
