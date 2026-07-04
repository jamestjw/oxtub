use tempfile::NamedTempFile;

use crate::{
    buffer::bpm::BufferPoolManager,
    catalog::{column::Column, manager::Catalog, schema::Schema, types::SqlType},
    query::{
        binder::transformer::Binder,
        engine::{QueryEngine, QueryResult},
        executor::{ExecutionEngine, engine::ExecutionResult},
        parser::parse_sql,
        planner::transformer::Planner,
    },
    storage::{
        disk::disk_manager::DiskManager,
        table::tuple::{Tuple, TupleMeta},
    },
    types::value::Value,
};

#[derive(Debug)]
enum SltRecord {
    Query {
        rowsort: bool,
        sql: String,
        expected: String,
    },
    StatementOk {
        sql: String,
    },
}

fn setup_bpm(pool_size: usize) -> BufferPoolManager {
    let file = NamedTempFile::new().unwrap();
    let disk_manager = DiskManager::new(file.path().to_path_buf()).unwrap();
    BufferPoolManager::new(pool_size, disk_manager)
}

fn setup_seqscan_catalog<'bpm>(bpm: &'bpm BufferPoolManager) -> Catalog<'bpm> {
    let mut catalog = Catalog::new(bpm);

    let schema = Schema::new(&[Column::new_static("col1".to_string(), SqlType::Integer)]);
    catalog
        .create_tbl("test_simple_seq_1".to_string(), schema.clone())
        .unwrap();
    let table = catalog.get_tbl_by_name("test_simple_seq_1").unwrap();
    for i in 0..10 {
        let tuple = Tuple::from_values(&[Value::Integer(i)], &schema);
        table
            .table_heap
            .insert_tuple(&TupleMeta::new(0, false), &tuple)
            .unwrap();
    }

    let schema = Schema::new(&[
        Column::new_static("col1".to_string(), SqlType::Integer),
        Column::new_static("col2".to_string(), SqlType::Integer),
    ]);
    catalog
        .create_tbl("test_simple_seq_2".to_string(), schema.clone())
        .unwrap();
    let table = catalog.get_tbl_by_name("test_simple_seq_2").unwrap();
    for i in 0..10 {
        let tuple = Tuple::from_values(&[Value::Integer(i), Value::Integer(i + 10)], &schema);
        table
            .table_heap
            .insert_tuple(&TupleMeta::new(0, false), &tuple)
            .unwrap();
    }

    catalog
}

fn execute_sql(catalog: &Catalog<'_>, sql: &str) -> ExecutionResult {
    let statement = parse_sql(sql).unwrap();
    let binder = Binder::new(catalog);
    let bound = binder.bind_statement(statement).unwrap();
    let planner = Planner::new(catalog);
    let plan = planner.plan_statement(bound).unwrap();
    let engine = ExecutionEngine::new(catalog);

    engine.execute(plan, 128).unwrap()
}

fn format_result(result: &ExecutionResult) -> String {
    result
        .rows
        .iter()
        .map(|row| {
            row.values
                .iter()
                .map(format_value)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_value(value: &Value) -> String {
    match value {
        Value::Boolean(value) => value.to_string(),
        Value::SmallInt(value) => value.to_string(),
        Value::Integer(value) => value.to_string(),
        Value::BigInt(value) => value.to_string(),
        Value::Decimal(value) => value.to_string(),
        Value::Varchar(value) => value.clone(),
        Value::Null(_) => "NULL".to_string(),
    }
}

fn parse_slt(script: &str) -> Vec<SltRecord> {
    // SQLLogicTest is line-oriented: a record starts with a header like
    // `query` or `query rowsort`, SQL follows until `----`, then expected
    // output follows until a blank line.
    let lines = script.lines().collect::<Vec<_>>();
    let mut records = vec![];
    let mut idx = 0;

    while idx < lines.len() {
        let line = lines[idx].trim_end();
        // Blank lines separate records. Comments can appear between records.
        if line.is_empty() || line.starts_with('#') {
            idx += 1;
            continue;
        }

        let tokens = line.split_whitespace().collect::<Vec<_>>();
        idx += 1;

        match tokens.as_slice() {
            ["query"] | ["query", "rowsort"] => {
                let rowsort = tokens.get(1).is_some_and(|token| *token == "rowsort");
                let mut sql = String::new();
                while idx < lines.len() && lines[idx] != "----" {
                    if !sql.is_empty() {
                        sql.push('\n');
                    }
                    sql.push_str(lines[idx]);
                    idx += 1;
                }
                assert!(idx < lines.len(), "query record missing result separator");
                idx += 1;

                let mut expected = String::new();
                while idx < lines.len() && !lines[idx].is_empty() {
                    if !expected.is_empty() {
                        expected.push('\n');
                    }
                    expected.push_str(lines[idx].trim_end());
                    idx += 1;
                }

                records.push(SltRecord::Query {
                    rowsort,
                    sql,
                    expected,
                });
            }
            ["statement", "ok"] => {
                let mut sql = String::new();
                while idx < lines.len() && !lines[idx].is_empty() {
                    if !sql.is_empty() {
                        sql.push('\n');
                    }
                    sql.push_str(lines[idx]);
                    idx += 1;
                }

                records.push(SltRecord::StatementOk { sql });
            }
            _ => panic!("unsupported SLT record header: {line}"),
        }
    }

    records
}

fn normalize_result(result: String, rowsort: bool) -> String {
    if !rowsort {
        return result;
    }

    let mut rows = result.lines().collect::<Vec<_>>();
    rows.sort_unstable();
    rows.join("\n")
}

#[test]
fn seqscan_slt() {
    let bpm = setup_bpm(10);
    let catalog = setup_seqscan_catalog(&bpm);
    let records = parse_slt(include_str!("../../test/sql/01-seqscan.slt"));

    for record in records {
        let SltRecord::Query {
            rowsort,
            sql,
            expected,
        } = record
        else {
            panic!("seqscan SLT only supports query records");
        };
        let actual = format_result(&execute_sql(&catalog, &sql));
        assert_eq!(
            normalize_result(expected, rowsort),
            normalize_result(actual, rowsort),
            "query failed:\n{}",
            sql
        );
    }
}

#[test]
fn insert_slt() {
    let bpm = setup_bpm(10);
    let mut catalog = setup_seqscan_catalog(&bpm);
    let records = parse_slt(include_str!("../../test/sql/02-insert.slt"));
    let mut engine = QueryEngine::new(&mut catalog);

    for record in records {
        match record {
            SltRecord::Query {
                rowsort,
                sql,
                expected,
            } => {
                let QueryResult::Rows(result) = engine.execute_sql(&sql).unwrap() else {
                    panic!("query record did not return rows:\n{sql}");
                };
                let actual = format_result(&result);
                assert_eq!(
                    normalize_result(expected, rowsort),
                    normalize_result(actual, rowsort),
                    "query failed:\n{}",
                    sql
                );
            }
            SltRecord::StatementOk { sql } => {
                let QueryResult::Command { .. } = engine.execute_sql(&sql).unwrap() else {
                    panic!("statement ok record did not return a command result:\n{sql}");
                };
            }
        }
    }
}
