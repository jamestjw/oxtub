use crate::{
    catalog::{manager::Catalog, schema::Schema},
    query::{
        executor::{
            delete::DeleteExecutor,
            error::ExecutionError,
            executor::{Executor, ExecutorContext},
            filter::FilterExecutor,
            insert::InsertExecutor,
            projection::ProjectionExecutor,
            seq_scan::SeqScanExecutor,
            update::UpdateExecutor,
            values::ValuesExecutor,
        },
        planner::plan::{PlanNode, PlanNodeKind},
    },
    storage::rid::Rid,
    types::value::Value,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutorRow {
    pub rid: Option<Rid>,
    pub values: Vec<Value>,
}

#[derive(Debug)]
pub struct ExecutionResult {
    pub schema: Schema,
    pub rows: Vec<ExecutorRow>,
}

pub struct ExecutionEngine<'catalog, 'bpm> {
    exec_ctx: ExecutorContext<'catalog, 'bpm>,
}

impl<'catalog, 'bpm> ExecutionEngine<'catalog, 'bpm> {
    pub fn new(catalog: &'catalog Catalog<'bpm>) -> Self {
        Self {
            exec_ctx: ExecutorContext::new(catalog),
        }
    }

    pub fn execute(
        &self,
        plan: PlanNode,
        batch_size: usize,
    ) -> Result<ExecutionResult, ExecutionError> {
        let mut executor = self.create_executor(&plan)?;
        executor.init()?;

        let mut rows = Vec::new();
        loop {
            let batch = executor.next(batch_size)?;
            if batch.is_empty() {
                break;
            }
            rows.extend(batch);
        }

        Ok(ExecutionResult {
            schema: plan.output_schema().clone(),
            rows,
        })
    }

    fn create_executor<'plan>(
        &'plan self,
        plan: &'plan PlanNode,
    ) -> Result<Box<dyn Executor + 'plan>, ExecutionError> {
        match &plan.kind {
            PlanNodeKind::SeqScan(seq_scan) => Ok(Box::new(SeqScanExecutor::new(
                &self.exec_ctx,
                seq_scan,
                plan.output_schema(),
            ))),
            PlanNodeKind::Filter(filter) => {
                let child = self.create_executor(&filter.child)?;
                Ok(Box::new(FilterExecutor::new(
                    filter,
                    plan.output_schema(),
                    child,
                )))
            }
            PlanNodeKind::Projection(projection) => {
                let child = self.create_executor(&projection.child)?;
                Ok(Box::new(ProjectionExecutor::new(
                    projection,
                    plan.output_schema(),
                    child,
                )))
            }
            PlanNodeKind::Values(values) => {
                Ok(Box::new(ValuesExecutor::new(values, plan.output_schema())))
            }
            PlanNodeKind::Insert(insert) => {
                let child = self.create_executor(&insert.child)?;
                Ok(Box::new(InsertExecutor::new(
                    &self.exec_ctx,
                    insert,
                    plan.output_schema(),
                    child,
                )))
            }
            PlanNodeKind::Delete(delete) => {
                let child = self.create_executor(&delete.child)?;
                Ok(Box::new(DeleteExecutor::new(
                    &self.exec_ctx,
                    delete,
                    plan.output_schema(),
                    child,
                )))
            }
            PlanNodeKind::Update(update) => {
                let child = self.create_executor(&update.child)?;
                Ok(Box::new(UpdateExecutor::new(
                    &self.exec_ctx,
                    update,
                    plan.output_schema(),
                    child,
                )))
            }
            PlanNodeKind::CreateTable(_) => Err(ExecutionError::UnsupportedPlan),
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::NamedTempFile;

    use crate::{
        buffer::bpm::BufferPoolManager,
        catalog::{column::Column, manager::Catalog, schema::Schema, types::SqlType},
        query::{binder::transformer::Binder, parser::parse_sql, planner::transformer::Planner},
        storage::disk::disk_manager::DiskManager,
        types::value::Value,
    };

    use super::*;

    fn setup_bpm(pool_size: usize) -> BufferPoolManager {
        let file = NamedTempFile::new().unwrap();
        let disk_manager = DiskManager::new(file.path().to_path_buf()).unwrap();
        BufferPoolManager::new(pool_size, disk_manager)
    }

    fn create_users_table(catalog: &mut Catalog<'_>) {
        create_table(catalog, "users");
    }

    fn create_table(catalog: &mut Catalog<'_>, name: &str) {
        let schema = Schema::new(&[
            Column::new_static("id".to_string(), SqlType::Integer),
            Column::new_variable("name".to_string(), SqlType::Varchar, 32),
        ]);

        catalog.create_tbl(name.to_string(), schema).unwrap();
    }

    fn execute_sql(catalog: &Catalog<'_>, sql: &str) -> ExecutionResult {
        let statement = parse_sql(sql).unwrap();
        let binder = Binder::new(catalog);
        let bound = binder.bind_statement(statement).unwrap();
        let planner = Planner::new(catalog);
        let plan = planner.plan_statement(bound).unwrap();
        let engine = ExecutionEngine::new(catalog);

        engine.execute(plan, 2).unwrap()
    }

    #[test]
    fn inserts_values_and_returns_count() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);

        let result = execute_sql(
            &catalog,
            "insert into users (id, name) values (1, 'alice'), (2, 'bob')",
        );

        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].values, vec![Value::Integer(2)]);

        let result = execute_sql(&catalog, "select id, name from users");
        assert_eq!(
            result.rows,
            vec![
                ExecutorRow {
                    rid: None,
                    values: vec![Value::Integer(1), Value::Varchar("alice".to_string())],
                },
                ExecutorRow {
                    rid: None,
                    values: vec![Value::Integer(2), Value::Varchar("bob".to_string())],
                },
            ]
        );
    }

    #[test]
    fn inserts_values_using_target_column_order() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);

        execute_sql(&catalog, "insert into users (name, id) values ('alice', 1)");

        let result = execute_sql(&catalog, "select id, name from users");
        assert_eq!(
            result.rows,
            vec![ExecutorRow {
                rid: None,
                values: vec![Value::Integer(1), Value::Varchar("alice".to_string())],
            }]
        );
    }

    #[test]
    fn inserts_rows_from_select() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_table(&mut catalog, "t1");
        create_table(&mut catalog, "t2");

        execute_sql(
            &catalog,
            "insert into t1 (id, name) values (1, 'alice'), (2, 'bob')",
        );
        let result = execute_sql(&catalog, "insert into t2 select * from t1");

        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].values, vec![Value::Integer(2)]);

        let result = execute_sql(&catalog, "select id, name from t2");
        assert_eq!(
            result.rows,
            vec![
                ExecutorRow {
                    rid: None,
                    values: vec![Value::Integer(1), Value::Varchar("alice".to_string())],
                },
                ExecutorRow {
                    rid: None,
                    values: vec![Value::Integer(2), Value::Varchar("bob".to_string())],
                },
            ]
        );
    }
}
