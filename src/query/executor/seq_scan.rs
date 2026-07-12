use crate::{
    catalog::schema::Schema,
    query::{
        executor::{
            engine::ExecutorRow,
            error::ExecutionError,
            executor::{Executor, ExecutorContext},
            expression::filter_keep_row,
        },
        planner::plan::SeqScanPlan,
    },
    storage::table::table_heap::TableHeapIterator,
};

pub struct SeqScanExecutor<'ctx, 'catalog, 'bpm, 'plan> {
    exec_ctx: &'ctx ExecutorContext<'catalog, 'bpm>,
    plan: &'plan SeqScanPlan,
    output_schema: &'plan Schema,
    table_iterator: Option<TableHeapIterator<'bpm>>,
}

impl<'ctx, 'catalog, 'bpm, 'plan> SeqScanExecutor<'ctx, 'catalog, 'bpm, 'plan> {
    pub fn new(
        exec_ctx: &'ctx ExecutorContext<'catalog, 'bpm>,
        plan: &'plan SeqScanPlan,
        output_schema: &'plan Schema,
    ) -> Self {
        Self {
            exec_ctx,
            plan,
            output_schema,
            table_iterator: None,
        }
    }
}

impl Executor for SeqScanExecutor<'_, '_, '_, '_> {
    fn init(&mut self) -> Result<(), ExecutionError> {
        let tbl_info = self.exec_ctx.catalog.get_tbl_by_oid(self.plan.table_oid)?;
        self.table_iterator = Some(tbl_info.table_heap.iter());

        Ok(())
    }

    fn next(&mut self, batch_size: usize) -> Result<Vec<ExecutorRow>, ExecutionError> {
        let Some(table_iterator) = self.table_iterator.as_mut() else {
            return Ok(vec![]);
        };

        let mut res = vec![];

        while res.len() < batch_size {
            let Some((rid, meta, tuple)) = table_iterator.next() else {
                break;
            };

            if meta.is_deleted() {
                continue;
            }

            let row = ExecutorRow {
                rid: Some(rid),
                values: tuple.get_values(self.output_schema),
            };

            if let Some(filter_predicate) = &self.plan.filter_predicate {
                if !filter_keep_row(filter_predicate, &row)? {
                    continue;
                }
            }

            res.push(row);
        }

        Ok(res)
    }

    fn output_schema(&self) -> &Schema {
        self.output_schema
    }
}
