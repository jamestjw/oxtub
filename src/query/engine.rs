use thiserror::Error;

use crate::{
    catalog::{
        column::Column, error::CatalogError, manager::Catalog, schema::Schema, types::SqlType,
    },
    query::{
        binder::{
            error::BinderError,
            statement::{BoundCreateIndex, BoundCreateTable, BoundExplain, BoundStatement},
            transformer::Binder,
        },
        error::QueryError,
        executor::{ExecutionEngine, ExecutorRow, engine::ExecutionResult, error::ExecutionError},
        optimizer::Optimizer,
        parser::parse_sql,
        planner::{error::PlannerError, format::format_plan, transformer::Planner},
    },
    types::value::Value,
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

    #[error("unique indexes are not supported")]
    UnsupportedUniqueIndex,
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
            BoundStatement::CreateIndex(create_index) => self.execute_create_index(create_index),
            BoundStatement::Explain(explain) => self.execute_explain(explain),
            BoundStatement::Select(_)
            | BoundStatement::Insert(_)
            | BoundStatement::Update(_)
            | BoundStatement::Delete(_) => {
                let plan = Planner::new(self.catalog).plan_statement(bound_statement)?;
                let plan = Optimizer::new(self.catalog).optimize(&plan);
                let execution_engine = ExecutionEngine::new(self.catalog);

                Ok(QueryResult::Rows(
                    execution_engine.execute(plan, batch_size)?,
                ))
            }
            BoundStatement::DropTable(_) | BoundStatement::DropIndex(_) => {
                Err(QueryEngineError::UnsupportedStatement)
            }
        }
    }

    fn execute_explain(
        &self,
        BoundExplain { raw, statement }: BoundExplain,
    ) -> Result<QueryResult, QueryEngineError> {
        let statement = *statement;
        let plan = match statement {
            BoundStatement::Select(_)
            | BoundStatement::Insert(_)
            | BoundStatement::Update(_)
            | BoundStatement::Delete(_) => Planner::new(self.catalog).plan_statement(statement)?,
            BoundStatement::Explain(_)
            | BoundStatement::CreateTable(_)
            | BoundStatement::CreateIndex(_)
            | BoundStatement::DropTable(_)
            | BoundStatement::DropIndex(_) => return Err(QueryEngineError::UnsupportedStatement),
        };

        let plan = if raw {
            plan
        } else {
            Optimizer::new(self.catalog).optimize(&plan)
        };
        let plan = format_plan(&plan);
        let schema = Schema::new(&[Column::new_variable(
            "QUERY PLAN".to_string(),
            SqlType::Varchar,
            plan.len(),
        )]);

        Ok(QueryResult::Rows(ExecutionResult {
            schema,
            rows: vec![ExecutorRow {
                rid: None,
                values: vec![Value::Varchar(plan)],
            }],
        }))
    }

    fn execute_create_table(
        &mut self,
        BoundCreateTable {
            name,
            columns,
            primary_key_col_idxs,
        }: BoundCreateTable,
    ) -> Result<QueryResult, QueryEngineError> {
        let table_name = name.clone();
        self.catalog.create_tbl(name, Schema::new(&columns))?;

        if !primary_key_col_idxs.is_empty() {
            let index_name = format!("{table_name}_pk");
            let key_attrs = primary_key_col_idxs;
            let key_columns = key_attrs
                .iter()
                .map(|idx| columns[*idx].clone())
                .collect::<Vec<_>>();
            let key_schema = Schema::new(&key_columns);
            let key_size = key_columns
                .iter()
                .map(|column| column.declared_size())
                .sum::<usize>();

            self.catalog.create_index(
                index_name, table_name, key_schema, key_attrs, key_size, true,
            )?;
        }

        Ok(QueryResult::Command {
            tag: String::from("CREATE TABLE"),
        })
    }

    fn execute_create_index(
        &mut self,
        BoundCreateIndex {
            index_name,
            table_name,
            key_schema,
            key_attrs,
            key_size,
            unique,
        }: BoundCreateIndex,
    ) -> Result<QueryResult, QueryEngineError> {
        if unique {
            return Err(QueryEngineError::UnsupportedUniqueIndex);
        }

        self.catalog.create_index(
            index_name, table_name, key_schema, key_attrs, key_size, false,
        )?;

        Ok(QueryResult::Command {
            tag: String::from("CREATE INDEX"),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        catalog::manager::Catalog,
        query::{engine::QueryEngine, engine::QueryEngineError, engine::QueryResult},
        testing::setup_bpm,
        types::value::Value,
    };

    fn setup_engine<'catalog, 'bpm>(
        catalog: &'catalog mut Catalog<'bpm>,
    ) -> QueryEngine<'catalog, 'bpm> {
        let mut engine = QueryEngine::new(catalog);
        engine
            .execute_sql("create table users(id int, name varchar(32));")
            .unwrap();
        engine
    }

    fn explain_plan(engine: &mut QueryEngine<'_, '_>, sql: &str) -> String {
        let QueryResult::Rows(result) = engine.execute_sql(sql).unwrap() else {
            panic!("EXPLAIN should return rows");
        };
        let Value::Varchar(plan) = &result.rows[0].values[0] else {
            panic!("EXPLAIN should return a VARCHAR plan");
        };

        plan.clone()
    }

    #[test]
    fn explain_returns_optimized_plan_by_default() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        let mut engine = setup_engine(&mut catalog);

        let plan = explain_plan(
            &mut engine,
            "explain select id, name from users where id = 1",
        );

        assert!(plan.contains("SeqScan table=users filter=(#0.0 = 1)"));
        assert!(!plan.contains("Filter predicate="));
    }

    #[test]
    fn explain_raw_returns_unoptimized_plan() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        let mut engine = setup_engine(&mut catalog);

        let plan = explain_plan(
            &mut engine,
            "explain (raw) select id, name from users where id = 1",
        );

        assert!(plan.contains("Filter predicate=(#0.0 = 1)"));
        assert!(plan.contains("SeqScan table=users"));
        assert!(!plan.contains("SeqScan table=users filter="));
    }

    #[test]
    fn executes_create_index() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        let mut engine = setup_engine(&mut catalog);

        let QueryResult::Command { tag } = engine
            .execute_sql("create index idx_users_id on users (id)")
            .unwrap()
        else {
            panic!("CREATE INDEX should return a command result");
        };

        assert_eq!(tag, "CREATE INDEX");
    }

    #[test]
    fn rejects_unique_create_index_at_execution() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        let mut engine = setup_engine(&mut catalog);
        let err = engine
            .execute_sql("create unique index idx_users_id on users (id)")
            .unwrap_err();

        assert!(matches!(err, QueryEngineError::UnsupportedUniqueIndex));
    }
}
