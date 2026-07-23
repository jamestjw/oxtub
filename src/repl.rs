use std::{
    fs,
    io::{self, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    buffer::bpm::BufferPoolManager, database::Database, query::engine::QueryResult,
    storage::disk::disk_manager::DiskManager, types::value::Value,
};

pub fn run() -> io::Result<()> {
    let db_path = TempDbPath::new();
    let disk_manager = DiskManager::new(db_path.path.clone()).unwrap();
    let bpm = BufferPoolManager::new(128, disk_manager);
    let database = Database::new(&bpm);
    let mut sql = String::new();

    // TODO: Handle Ctrl+C/SIGINT gracefully so the temp DB file is cleaned up.
    loop {
        print_prompt(sql.is_empty())?;

        let mut line = String::new();
        if io::stdin().read_line(&mut line)? == 0 {
            break;
        }

        // can only accept special commands when there is no input so far
        if sql.is_empty() {
            match line.trim() {
                ".exit" | ".quit" => break,
                ".help" => {
                    print_help();
                    continue;
                }
                ".tables" => {
                    if let Err(err) = print_tables(&database) {
                        eprintln!("error: {err}");
                    }
                    continue;
                }
                "" => continue,
                _ => {}
            }
        }

        sql.push_str(&line);
        if !sql.trim_end().ends_with(';') {
            continue;
        }

        let statement = sql.trim().trim_end_matches(';').trim();
        if !statement.is_empty() {
            match database.execute_sql(statement) {
                Ok(result) => print_result(result),
                Err(err) => eprintln!("error: {err}"),
            }
        }

        sql.clear();
    }

    drop(database);
    drop(bpm);

    Ok(())
}

fn print_prompt(is_start_of_statement: bool) -> io::Result<()> {
    print!(
        "{}",
        if is_start_of_statement {
            "oxtub> "
        } else {
            "   ...> "
        }
    );
    io::stdout().flush()
}

fn temp_db_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    std::env::temp_dir().join(format!("oxtub-{}-{nanos}.db", std::process::id()))
}

struct TempDbPath {
    path: PathBuf,
}

impl TempDbPath {
    fn new() -> Self {
        Self {
            path: temp_db_path(),
        }
    }
}

impl Drop for TempDbPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn print_help() {
    println!("Enter SQL terminated by a semicolon.");
    println!("Commands: .help, .tables, .quit, .exit");
}

fn print_tables(database: &Database<'_>) -> Result<(), crate::database::DatabaseError> {
    let mut table_names = database.table_names()?;
    table_names.sort_unstable();

    if table_names.is_empty() {
        println!("(no tables)");
        return Ok(());
    }

    for table_name in table_names {
        println!("{table_name}");
    }

    Ok(())
}

fn print_result(result: QueryResult) {
    match result {
        QueryResult::Command { tag } => println!("{tag}"),
        QueryResult::Rows(result) => {
            for row in result.rows {
                println!(
                    "{}",
                    row.values
                        .iter()
                        .map(format_value)
                        .collect::<Vec<_>>()
                        .join(" ")
                );
            }
        }
    }
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
