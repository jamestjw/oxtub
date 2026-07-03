use thiserror::Error;

use crate::{
    catalog::{error::CatalogError, manager::Catalog, schema::Schema},
    query::{
        binder::{
            error::BinderError,
            statement::{BoundCreateTable, BoundStatement},
            transformer::Binder,
        },
        error::QueryError,
        executor::{engine::ExecutionResult, error::ExecutionError},
        parser::parse_sql,
        planner::{error::PlannerError, transformer::Planner},
    },
};

const DEFAULT_BATCH_SIZE: usize = 128;

#[derive(Debug, Error)]
pub enum QueryEngineError {
    #[error("query error: {0}")]
    Query(#[from] QueryError),

    #[error("binder error: {0}")]
    Binder(#[from] BinderError),

    #[error("planner error: {0}")]
    Planner(#[from] PlannerError),

    #[error("execution error: {0}")]
    Execution(#[from] ExecutionError),

    #[error("catalog error: {0}")]
    Catalog(#[from] CatalogError),

    #[error("unsupported statement")]
    UnsupportedStatement,
}

#[derive(Debug)]
pub enum QueryResult {
    Rows(ExecutionResult),
    Command { tag: String },
}

pub struct QueryEngine<'catalog, 'bpm> {
    catalog: &'catalog mut Catalog<'bpm>,
}

impl<'catalog, 'bpm> QueryEngine<'catalog, 'bpm> {
    pub fn new(catalog: &'catalog mut Catalog<'bpm>) -> Self {
        Self { catalog }
    }

    pub fn execute_sql(&mut self, sql: &str) -> Result<QueryResult, QueryEngineError> {
        self.execute_sql_with_batch_size(sql, DEFAULT_BATCH_SIZE)
    }

    pub fn execute_sql_with_batch_size(
        &mut self,
        sql: &str,
        batch_size: usize,
    ) -> Result<QueryResult, QueryEngineError> {
        let statement = parse_sql(sql)?;
        let bound_statement = Binder::new(self.catalog).bind_statement(statement)?;

        match bound_statement {
            BoundStatement::CreateTable(create_table) => self.execute_create_table(create_table),
            BoundStatement::Select(_)
            | BoundStatement::Insert(_)
            | BoundStatement::Update(_)
            | BoundStatement::Delete(_) => {
                let plan = Planner::new(self.catalog).plan_statement(bound_statement)?;
                let execution_engine =
                    crate::query::executor::engine::ExecutionEngine::new(self.catalog);

                Ok(QueryResult::Rows(
                    execution_engine.execute(plan, batch_size)?,
                ))
            }
            BoundStatement::Explain(_)
            | BoundStatement::CreateIndex(_)
            | BoundStatement::DropTable(_)
            | BoundStatement::DropIndex(_) => Err(QueryEngineError::UnsupportedStatement),
        }
    }

    fn execute_create_table(
        &mut self,
        BoundCreateTable {
            name,
            columns,
            primary_key_cols,
        }: BoundCreateTable,
    ) -> Result<QueryResult, QueryEngineError> {
        let tbl_info = self.catalog.create_tbl(name, Schema::new(&columns))?;

        Ok(QueryResult::Command {
            tag: String::from("CREATE TABLE"),
        })
    }
}
