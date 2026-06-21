use std::collections::HashSet;

use crate::{
    catalog::{column::Column, types::SqlType},
    query::{
        binder::{
            error::BinderError,
            statement::{BoundCreateTable, BoundStatement},
        },
        statement::{CreateColumn, CreateTableStatement, Statement},
    },
};

pub fn bind_statement(stmt: Statement) -> Result<BoundStatement, BinderError> {
    match stmt {
        Statement::Select(select_statement) => todo!(),
        Statement::Insert(insert_statement) => todo!(),
        Statement::CreateTable(create_table_statement) => bind_create(create_table_statement),
    }
}

fn bind_create(stmt: CreateTableStatement) -> Result<BoundStatement, BinderError> {
    let mut seen_columns = HashSet::new();
    let mut columns = Vec::with_capacity(stmt.columns.len());

    for column in stmt.columns {
        let column_key = column.name.to_lowercase();
        if !seen_columns.insert(column_key) {
            return Err(BinderError::DuplicateColumn(column.name));
        }

        columns.push(bind_create_column(column));
    }

    validate_primary_key(&stmt.primary_key, &columns)?;

    Ok(BoundStatement::CreateTable(BoundCreateTable {
        name: stmt.table_name,
        columns,
        primary_key_cols: stmt.primary_key,
    }))
}

fn bind_create_column(column: CreateColumn) -> Column {
    match column.sql_type {
        SqlType::Varchar => {
            let size = column
                .size
                .expect("parser produced VARCHAR column without size");
            Column::new_variable(column.name, SqlType::Varchar, size)
        }
        sql_type => {
            assert!(
                column.size.is_none(),
                "parser produced non-VARCHAR column with size"
            );
            Column::new_static(column.name, sql_type)
        }
    }
}

fn validate_primary_key(primary_key: &[String], columns: &[Column]) -> Result<(), BinderError> {
    let column_names = columns
        .iter()
        .map(|column| column.name().to_lowercase())
        .collect::<HashSet<_>>();
    let mut seen_primary_key_cols = HashSet::new();

    for primary_key_col in primary_key {
        let primary_key_col_key = primary_key_col.to_lowercase();

        if !seen_primary_key_cols.insert(primary_key_col_key.clone()) {
            return Err(BinderError::DuplicatePrimaryKeyColumn(
                primary_key_col.clone(),
            ));
        }

        if !column_names.contains(&primary_key_col_key) {
            return Err(BinderError::PrimaryKeyColumnNotFound(
                primary_key_col.clone(),
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use expect_test::expect;

    use crate::query::{binder::statement::BoundStatement, parser::parse_sql};

    use super::*;

    fn unwrap_binder_err(result: Result<BoundStatement, BinderError>) -> BinderError {
        match result {
            Ok(_) => panic!("expected binder error"),
            Err(err) => err,
        }
    }

    #[test]
    fn binds_create_table_columns() {
        let statement = parse_sql("create table users (id integer, name varchar(32))").unwrap();

        let bound = bind_statement(statement).unwrap();
        let BoundStatement::CreateTable(create_table) = bound else {
            panic!("expected create table statement");
        };

        expect![[r#"
            BoundCreateTable {
                name: "users",
                columns: [
                    Column {
                        name: "id",
                        sql_type: Integer,
                        value_offset: 0,
                        size: Inline(
                            4,
                        ),
                    },
                    Column {
                        name: "name",
                        sql_type: Varchar,
                        value_offset: 0,
                        size: Variable(
                            32,
                        ),
                    },
                ],
                primary_key_cols: [],
            }"#]]
        .assert_eq(&format!("{create_table:#?}"));
    }

    #[test]
    fn binds_create_table_primary_key() {
        let statement = parse_sql(
            "create table users (tenant_id integer, id integer, primary key (tenant_id, id))",
        )
        .unwrap();

        let bound = bind_statement(statement).unwrap();
        let BoundStatement::CreateTable(create_table) = bound else {
            panic!("expected create table statement");
        };

        expect![[r#"
            BoundCreateTable {
                name: "users",
                columns: [
                    Column {
                        name: "tenant_id",
                        sql_type: Integer,
                        value_offset: 0,
                        size: Inline(
                            4,
                        ),
                    },
                    Column {
                        name: "id",
                        sql_type: Integer,
                        value_offset: 0,
                        size: Inline(
                            4,
                        ),
                    },
                ],
                primary_key_cols: [
                    "tenant_id",
                    "id",
                ],
            }"#]]
        .assert_eq(&format!("{create_table:#?}"));
    }

    #[test]
    fn rejects_duplicate_create_table_columns() {
        let statement = parse_sql("create table users (id integer, id integer)").unwrap();
        let err = unwrap_binder_err(bind_statement(statement));

        assert!(matches!(err, BinderError::DuplicateColumn(column) if column == "id"));
    }

    #[test]
    fn rejects_missing_primary_key_column() {
        let statement =
            parse_sql("create table users (id integer, primary key (missing))").unwrap();
        let err = unwrap_binder_err(bind_statement(statement));

        assert!(
            matches!(err, BinderError::PrimaryKeyColumnNotFound(column) if column == "missing")
        );
    }

    #[test]
    fn rejects_duplicate_primary_key_column() {
        let statement = parse_sql("create table users (id integer, primary key (id, id))").unwrap();
        let err = unwrap_binder_err(bind_statement(statement));

        assert!(matches!(err, BinderError::DuplicatePrimaryKeyColumn(column) if column == "id"));
    }
}
