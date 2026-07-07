use crate::{
    catalog::schema::Schema,
    query::{
        executor::{
            engine::ExecutorRow,
            error::ExecutionError,
            executor::{Executor, ExecutorContext},
        },
        planner::plan::InsertPlan,
    },
    storage::table::tuple::{Tuple, TupleMeta},
    types::value::Value,
};

pub struct InsertExecutor<'ctx, 'catalog, 'bpm, 'plan> {
    exec_ctx: &'ctx ExecutorContext<'catalog, 'bpm>,
    plan: &'plan InsertPlan,
    output_schema: &'plan Schema,
    child: Box<dyn Executor + 'plan>,
    done: bool,
}

impl<'ctx, 'catalog, 'bpm, 'plan> InsertExecutor<'ctx, 'catalog, 'bpm, 'plan> {
    pub fn new(
        exec_ctx: &'ctx ExecutorContext<'catalog, 'bpm>,
        plan: &'plan InsertPlan,
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

impl Executor for InsertExecutor<'_, '_, '_, '_> {
    fn init(&mut self) -> Result<(), ExecutionError> {
        self.done = false;
        self.child.init()
    }

    fn next(&mut self, batch_size: usize) -> Result<Vec<ExecutorRow>, ExecutionError> {
        if self.done {
            return Ok(vec![]);
        }

        let table_info = self.exec_ctx.catalog.get_tbl_by_oid(self.plan.table_oid)?;
        let indexes = self
            .exec_ctx
            .catalog
            .get_table_indexes(&self.plan.table_name)?;
        let mut inserted_count = 0;

        loop {
            let batch = self.child.next(batch_size)?;
            if batch.is_empty() {
                break;
            }

            for row in batch {
                let mut values = self
                    .plan
                    .table_schema
                    .columns()
                    .iter()
                    .map(|col| Value::Null(col.sql_type()))
                    .collect::<Vec<_>>();

                for (value, target_col_idx) in
                    row.values.into_iter().zip(&self.plan.target_col_idxs)
                {
                    values[*target_col_idx] = value;
                }

                let tuple = Tuple::from_values(&values, &self.plan.table_schema);
                let rid = table_info
                    .table_heap
                    .insert_tuple(&TupleMeta::new(0, false), &tuple)?;

                for index_info in indexes.iter() {
                    let metadata = index_info.index.metadata();
                    let key = tuple.key_from_tuple(
                        &self.plan.table_schema,
                        &metadata.key_schema,
                        &metadata.key_attrs,
                    );
                    index_info.index.insert_entry(&key, rid)?;
                }

                inserted_count += 1;
            }
        }

        self.done = true;

        Ok(vec![ExecutorRow {
            rid: None,
            values: vec![Value::Integer(inserted_count)],
        }])
    }

    fn output_schema(&self) -> &Schema {
        self.output_schema
    }
}
