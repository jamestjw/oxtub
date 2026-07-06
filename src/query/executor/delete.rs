use crate::{
    catalog::schema::Schema,
    query::{
        executor::{
            engine::ExecutorRow,
            error::ExecutionError,
            executor::{Executor, ExecutorContext},
        },
        planner::plan::DeletePlan,
    },
    types::value::Value,
};

pub struct DeleteExecutor<'ctx, 'catalog, 'bpm, 'plan> {
    exec_ctx: &'ctx ExecutorContext<'catalog, 'bpm>,
    plan: &'plan DeletePlan,
    output_schema: &'plan Schema,
    child: Box<dyn Executor + 'plan>,
    done: bool,
}

impl<'ctx, 'catalog, 'bpm, 'plan> DeleteExecutor<'ctx, 'catalog, 'bpm, 'plan> {
    pub fn new(
        exec_ctx: &'ctx ExecutorContext<'catalog, 'bpm>,
        plan: &'plan DeletePlan,
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

impl Executor for DeleteExecutor<'_, '_, '_, '_> {
    fn init(&mut self) -> Result<(), ExecutionError> {
        self.done = false;
        self.child.init()
    }

    fn next(&mut self, batch_size: usize) -> Result<Vec<ExecutorRow>, ExecutionError> {
        if self.done {
            return Ok(vec![]);
        }

        let mut deleted_count = 0;
        let table_info = self.exec_ctx.catalog.get_tbl_by_oid(self.plan.table_oid)?;
        let indexes = self
            .exec_ctx
            .catalog
            .get_table_indexes(&table_info.name())?;
        let table_schema = table_info.schema();

        loop {
            let batch = self.child.next(batch_size)?;
            if batch.is_empty() {
                break;
            }

            for ExecutorRow { rid, values: _ } in batch {
                let rid = rid.expect("delete executor child produced row without rid");
                let (old_tuple_meta, old_tuple) = table_info.table_heap.get_tuple(rid)?;
                assert!(!old_tuple_meta.is_deleted(), "tuple already deleted");

                for index_info in indexes.iter() {
                    let metadata = index_info.index.metadata();

                    let old_key = old_tuple.key_from_tuple(
                        &table_schema,
                        &metadata.key_schema,
                        &metadata.key_attrs,
                    );
                    index_info.index.delete_entry(&old_key, rid)?;
                }

                // mark tuple as deleted
                table_info
                    .table_heap
                    .update_meta(rid, old_tuple_meta.delete())?;

                deleted_count += 1;
            }
        }

        self.done = true;

        Ok(vec![ExecutorRow {
            rid: None,
            values: vec![Value::Integer(deleted_count)],
        }])
    }

    fn output_schema(&self) -> &Schema {
        self.output_schema
    }
}
