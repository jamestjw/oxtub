use std::sync::Mutex;

use thiserror::Error;

use crate::{
    buffer::bpm::BufferPoolManager,
    catalog::Catalog,
    query::engine::{QueryEngine, QueryEngineError, QueryResult},
};

/// The application-level database boundary.
///
/// Statements hold the catalog mutex from parsing through execution. This keeps
/// catalog and storage mutations serialized until transaction support exists.
pub struct Database<'bpm> {
    catalog: Mutex<Catalog<'bpm>>,
}

#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error(transparent)]
    Query(#[from] QueryEngineError),
}

impl<'bpm> Database<'bpm> {
    pub fn new(bpm: &'bpm BufferPoolManager) -> Self {
        Self {
            catalog: Mutex::new(Catalog::new(bpm)),
        }
    }

    pub fn execute_sql(&self, sql: &str) -> Result<QueryResult, DatabaseError> {
        let mut catalog = self.catalog.lock().unwrap();
        let mut engine = QueryEngine::new(&mut catalog);

        Ok(engine.execute_sql(sql)?)
    }

    pub fn table_names(&self) -> Result<Vec<String>, DatabaseError> {
        let catalog = self.catalog.lock().unwrap();
        Ok(catalog
            .table_names()
            .into_iter()
            .map(str::to_owned)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Barrier, thread};

    use crate::{
        catalog::error::CatalogError,
        query::engine::{QueryEngineError, QueryResult},
        testing::setup_bpm,
    };

    use super::*;

    #[test]
    fn concurrent_statements_are_serialized() {
        let bpm = setup_bpm(8);
        let database = Database::new(&bpm);
        let barrier = Barrier::new(2);

        let (first, second) = thread::scope(|scope| {
            let first = scope.spawn(|| {
                barrier.wait();
                database.execute_sql("create table users(id int)")
            });
            let second = scope.spawn(|| {
                barrier.wait();
                database.execute_sql("create table users(id int)")
            });

            (first.join().unwrap(), second.join().unwrap())
        });

        let results = [first, second];
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Ok(QueryResult::Command { .. })))
                .count(),
            1
        );
        assert_eq!(
            results
                .iter()
                .filter(|result| {
                    matches!(
                        result,
                        Err(DatabaseError::Query(QueryEngineError::Catalog(
                            CatalogError::DuplicateTable(name)
                        ))) if name == "users"
                    )
                })
                .count(),
            1
        );
        assert_eq!(database.table_names().unwrap(), vec!["users"]);
    }
}
