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
            PlanNodeKind::CreateTable(_) | PlanNodeKind::Update(_) => {
                Err(ExecutionError::UnsupportedPlan)
            }
        }
    }
}
