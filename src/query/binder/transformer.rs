use std::collections::HashSet;

use crate::{
    catalog::{column::Column, manager::Catalog, schema::Schema, types::SqlType},
    query::{
        binder::{
            error::BinderError,
            expression::{BoundExpression, ColumnRef, are_column_refs_unique},
            statement::{BoundCreateTable, BoundInsert, BoundSelect, BoundStatement},
            table_ref::{BoundBaseTableRef, BoundExpressionListRef, TableRef},
        },
        expression::Expression,
        statement::{
            CreateColumn, CreateTableStatement, InsertStatement, SelectItem, SelectStatement,
            Statement,
        },
    },
};

pub struct Binder<'catalog, 'bpm> {
    catalog: &'catalog Catalog<'bpm>,
}

struct BindContext<'a> {
    scope: Option<&'a TableRef>,
}

impl<'a> BindContext<'a> {
    fn no_scope() -> Self {
        Self { scope: None }
    }

    fn table_scope(table: &'a TableRef) -> Self {
        Self { scope: Some(table) }
    }
}

impl<'catalog, 'bpm> Binder<'catalog, 'bpm> {
    pub fn new(catalog: &'catalog Catalog<'bpm>) -> Self {
        Self { catalog }
    }

    pub fn bind_statement(&self, stmt: Statement) -> Result<BoundStatement, BinderError> {
        match stmt {
            Statement::Select(select_statement) => {
                let select = self.bind_select(select_statement)?;
                Ok(BoundStatement::Select(select))
            }
            Statement::Insert(insert_statement) => self.bind_insert(insert_statement),
            Statement::Update(_update_statement) => todo!(),
            Statement::Delete(_delete_statement) => todo!(),
            Statement::CreateTable(create_table_statement) => {
                self.bind_create_tbl(create_table_statement)
            }
        }
    }

    fn bind_select(&self, stmt: SelectStatement) -> Result<BoundSelect, BinderError> {
        // TODO: should support aliases!
        // TODO: can also select without a From clause
        let tbl_ref = TableRef::BaseTable(self.bind_base_table_ref(stmt.table_name, None)?);
        let context = BindContext::table_scope(&tbl_ref);
        let projection = self.bind_select_list(stmt.projection, &context)?;
        let where_ = match stmt.where_clause {
            None => None,
            Some(expr) => Some(self.bind_expression(expr, &context)?),
        };

        Ok(BoundSelect {
            table: tbl_ref,
            projection,
            where_,
        })
    }

    fn bind_select_list(
        &self,
        select_item: Vec<SelectItem>,
        context: &BindContext,
    ) -> Result<Vec<BoundExpression>, BinderError> {
        let mut res = vec![];

        for s in select_item {
            match s {
                SelectItem::Wildcard => {
                    res.extend(Self::get_all_cols_from_scope(context)?);
                }
                SelectItem::Expression(expression) => {
                    res.push(self.bind_expression(expression, context)?);
                }
            }
        }

        if res.is_empty() {
            return Err(BinderError::EmptySelectProjection);
        }

        Ok(res)
    }

    fn get_all_cols_from_scope(context: &BindContext) -> Result<Vec<BoundExpression>, BinderError> {
        match context.scope {
            None => Err(BinderError::UnsupportedExpression(
                "select * without table scope".into(),
            )),
            Some(TableRef::BaseTable(bound_base_table_ref)) => {
                let tbl_name = bound_base_table_ref.bound_tbl_name();
                let cols = bound_base_table_ref.schema().columns();
                let mut res = Vec::with_capacity(cols.len());
                for col in cols {
                    res.push(BoundExpression::Column(ColumnRef::TableQualified {
                        table: tbl_name.into(),
                        column: col.name().into(),
                    }));
                }

                Ok(res)
            }
            Some(TableRef::ExprList(_)) => panic!("select * should not use this table ref"),
        }
    }

    fn bind_create_tbl(&self, stmt: CreateTableStatement) -> Result<BoundStatement, BinderError> {
        let mut seen_columns = HashSet::new();
        let mut columns = Vec::with_capacity(stmt.columns.len());

        for column in stmt.columns {
            let column_key = column.name.to_lowercase();
            if !seen_columns.insert(column_key) {
                return Err(BinderError::DuplicateColumn(column.name));
            }

            columns.push(bind_create_column(column));
        }

        if columns.is_empty() {
            return Err(BinderError::CreateTableWithoutColumns);
        }

        validate_primary_key(&stmt.primary_key, &columns)?;

        Ok(BoundStatement::CreateTable(BoundCreateTable {
            name: stmt.table_name,
            columns,
            primary_key_cols: stmt.primary_key,
        }))
    }

    fn bind_insert(&self, stmt: InsertStatement) -> Result<BoundStatement, BinderError> {
        if stmt.table_name.starts_with("__") {
            return Err(BinderError::InvalidTableName(stmt.table_name));
        }

        let table = self.bind_base_table_ref(stmt.table_name.clone(), None)?;
        let columns = match stmt.columns {
            None => Self::bind_columns_from_schema(table.schema()),
            Some(columns) => {
                let columns = columns
                    .iter()
                    .map(|col| Self::resolve_column_ref_from_base_table_ref(&table, col.clone()))
                    .collect::<Result<Vec<_>, BinderError>>()?;

                if !are_column_refs_unique(&columns) {
                    return Err(BinderError::DuplicateInsertColumns);
                }
                columns
            }
        };

        let num_columns = columns.len();
        Ok(BoundStatement::Insert(BoundInsert {
            table,
            columns,
            bound_exprs: self.bind_values_list(num_columns, stmt.values)?,
        }))
    }

    fn bind_columns_from_schema(schema: &Schema) -> Vec<ColumnRef> {
        schema
            .columns()
            .iter()
            .map(|col| ColumnRef::Unqualified {
                column: col.name().to_string(),
            })
            .collect()
    }

    // Bind values from an insert statement
    fn bind_values_list(
        &self,
        num_cols: usize,
        rows: Vec<Vec<Expression>>,
    ) -> Result<BoundExpressionListRef, BinderError> {
        if rows.is_empty() {
            return Err(BinderError::InsertValuesEmpty);
        }

        let mut res = Vec::with_capacity(rows.len());
        let context = BindContext::no_scope();

        for row in rows {
            if row.len() != num_cols {
                return Err(BinderError::InsertValuesDoesntMatchColumns);
            }
            res.push(self.bind_expr_list(row, &context)?);
        }

        Ok(BoundExpressionListRef::new(String::from("<unnamed>"), res))
    }

    fn bind_expr_list(
        &self,
        exprs: Vec<Expression>,
        context: &BindContext<'_>,
    ) -> Result<Vec<BoundExpression>, BinderError> {
        let mut res = Vec::with_capacity(exprs.len());

        for expr in exprs {
            let expr = self.bind_expression(expr, context)?;

            res.push(expr);
        }

        Ok(res)
    }

    fn bind_expression(
        &self,
        expr: Expression,
        context: &BindContext<'_>,
    ) -> Result<BoundExpression, BinderError> {
        match expr {
            Expression::Literal(value) => Ok(BoundExpression::Literal(value)),
            Expression::Column(c) => Ok(BoundExpression::Column(self.bind_column_ref(c, context)?)),
            Expression::UnaryOp { op, expr } => Ok(BoundExpression::UnaryOp {
                op,
                expr: Box::new(self.bind_expression(*expr, context)?),
            }),
            Expression::BinaryOp { left, op, right } => Ok(BoundExpression::BinaryOp {
                left: Box::new(self.bind_expression(*left, context)?),
                op,
                right: Box::new(self.bind_expression(*right, context)?),
            }),
        }
    }

    // todo: handle column refs that are qualified
    // todo: when doing the above, handle the case where aliases can cause ambiguity
    fn bind_column_ref(
        &self,
        column: String,
        context: &BindContext<'_>,
    ) -> Result<ColumnRef, BinderError> {
        match context.scope {
            Some(TableRef::BaseTable(table)) => {
                Self::resolve_column_ref_from_base_table_ref(table, column)
            }
            Some(TableRef::ExprList(_)) => Err(BinderError::UnsupportedExpression(format!(
                "column reference `{column}` in expression list scope"
            ))),
            None => Err(BinderError::UnsupportedExpression(format!(
                "column reference `{column}` without table scope"
            ))),
        }
    }

    fn resolve_column_ref_from_base_table_ref(
        base_table_ref: &BoundBaseTableRef,
        col_name: String,
    ) -> Result<ColumnRef, BinderError> {
        match Self::resolve_column_ref_schema(base_table_ref.schema(), &col_name)? {
            None => Err(BinderError::ColumnNotFound(col_name)),
            Some(ColumnRef::Unqualified { column }) => Ok(ColumnRef::TableQualified {
                table: base_table_ref.bound_tbl_name().into(),
                column,
            }),
            Some(c) => Ok(c),
        }
    }

    fn resolve_column_ref_schema(
        schema: &Schema,
        col_name: &str,
    ) -> Result<Option<ColumnRef>, BinderError> {
        let mut res = None;
        for col in schema.columns() {
            if col.name().to_lowercase() == col_name.to_lowercase() {
                if res.is_some() {
                    return Err(BinderError::AmbiguousColumn(col_name.into()));
                }

                res = Some(ColumnRef::Unqualified {
                    column: String::from(col.name()),
                })
            }
        }

        Ok(res)
    }

    fn bind_base_table_ref(
        &self,
        table_name: String,
        alias: Option<String>,
    ) -> Result<BoundBaseTableRef, BinderError> {
        let table_info = self.catalog.get_tbl_by_name(&table_name)?;
        Ok(BoundBaseTableRef::new(
            table_name,
            table_info.table_oid(),
            alias,
            table_info.schema(),
        ))
    }
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
    use tempfile::NamedTempFile;

    use crate::{
        buffer::bpm::BufferPoolManager,
        catalog::{column::Column, schema::Schema},
        query::{binder::statement::BoundStatement, parser::parse_sql},
        storage::disk::disk_manager::DiskManager,
    };

    use super::*;

    fn unwrap_binder_err(result: Result<BoundStatement, BinderError>) -> BinderError {
        match result {
            Ok(_) => panic!("expected binder error"),
            Err(err) => err,
        }
    }

    fn setup_bpm(pool_size: usize) -> BufferPoolManager {
        let file = NamedTempFile::new().unwrap();
        let disk_manager = DiskManager::new(file.path().to_path_buf()).unwrap();
        BufferPoolManager::new(pool_size, disk_manager)
    }

    fn create_users_table(catalog: &mut Catalog<'_>) {
        let schema = Schema::new(&[
            Column::new_static("id".to_string(), SqlType::Integer),
            Column::new_variable("name".to_string(), SqlType::Varchar, 32),
        ]);

        catalog.create_tbl("users".to_string(), schema).unwrap();
    }

    #[test]
    fn binds_create_table_columns() {
        let bpm = setup_bpm(3);
        let catalog = Catalog::new(&bpm);
        let binder = Binder::new(&catalog);
        let statement = parse_sql("create table users (id integer, name varchar(32))").unwrap();

        let bound = binder.bind_statement(statement).unwrap();
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
        let bpm = setup_bpm(3);
        let catalog = Catalog::new(&bpm);
        let binder = Binder::new(&catalog);
        let statement = parse_sql(
            "create table users (tenant_id integer, id integer, primary key (tenant_id, id))",
        )
        .unwrap();

        let bound = binder.bind_statement(statement).unwrap();
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
    fn binds_insert_with_columns_and_literal_values() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);
        let binder = Binder::new(&catalog);
        let statement =
            parse_sql("insert into users (id, name) values (1, 'alice'), (2, 'bob')").unwrap();

        let bound = binder.bind_statement(statement).unwrap();
        let BoundStatement::Insert(insert) = bound else {
            panic!("expected insert statement");
        };

        expect![[r#"
            BoundInsert {
                table: BoundBaseTableRef {
                    table_name: "users",
                    table_oid: 0,
                    alias: None,
                    schema: Schema {
                        inlined_storage_size: 9,
                        columns: [
                            Column {
                                name: "id",
                                sql_type: Integer,
                                value_offset: 1,
                                size: Inline(
                                    4,
                                ),
                            },
                            Column {
                                name: "name",
                                sql_type: Varchar,
                                value_offset: 5,
                                size: Variable(
                                    32,
                                ),
                            },
                        ],
                        uninlined_columns: [
                            1,
                        ],
                    },
                },
                columns: [
                    TableQualified {
                        table: "users",
                        column: "id",
                    },
                    TableQualified {
                        table: "users",
                        column: "name",
                    },
                ],
                bound_exprs: BoundExpressionListRef {
                    identifier: "<unnamed>",
                    values: [
                        [
                            Literal(
                                Integer(
                                    1,
                                ),
                            ),
                            Literal(
                                Varchar(
                                    "alice",
                                ),
                            ),
                        ],
                        [
                            Literal(
                                Integer(
                                    2,
                                ),
                            ),
                            Literal(
                                Varchar(
                                    "bob",
                                ),
                            ),
                        ],
                    ],
                },
            }"#]]
        .assert_eq(&format!("{insert:#?}"));
    }

    #[test]
    fn binds_select_wildcard() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);
        let binder = Binder::new(&catalog);
        let statement = parse_sql("select * from users").unwrap();

        let bound = binder.bind_statement(statement).unwrap();
        let BoundStatement::Select(select) = bound else {
            panic!("expected select statement");
        };

        assert_eq!(
            select.projection,
            vec![
                BoundExpression::Column(ColumnRef::TableQualified {
                    table: "users".to_string(),
                    column: "id".to_string(),
                }),
                BoundExpression::Column(ColumnRef::TableQualified {
                    table: "users".to_string(),
                    column: "name".to_string(),
                }),
            ]
        );
        assert_eq!(select.where_, None);
    }

    #[test]
    fn binds_select_columns() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);
        let binder = Binder::new(&catalog);
        let statement = parse_sql("select id, name from users").unwrap();

        let bound = binder.bind_statement(statement).unwrap();
        let BoundStatement::Select(select) = bound else {
            panic!("expected select statement");
        };

        assert_eq!(
            select.projection,
            vec![
                BoundExpression::Column(ColumnRef::TableQualified {
                    table: "users".to_string(),
                    column: "id".to_string(),
                }),
                BoundExpression::Column(ColumnRef::TableQualified {
                    table: "users".to_string(),
                    column: "name".to_string(),
                }),
            ]
        );
        assert_eq!(select.where_, None);
    }

    #[test]
    fn binds_select_where_clause() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);
        let binder = Binder::new(&catalog);
        let statement = parse_sql("select id from users where id = 1").unwrap();

        let bound = binder.bind_statement(statement).unwrap();
        let BoundStatement::Select(select) = bound else {
            panic!("expected select statement");
        };

        assert_eq!(
            select.projection,
            vec![BoundExpression::Column(ColumnRef::TableQualified {
                table: "users".to_string(),
                column: "id".to_string(),
            })]
        );
        expect![[r#"
            Some(
                BinaryOp {
                    left: Column(
                        TableQualified {
                            table: "users",
                            column: "id",
                        },
                    ),
                    op: Eq,
                    right: Literal(
                        Integer(
                            1,
                        ),
                    ),
                },
            )"#]]
        .assert_eq(&format!("{:#?}", select.where_));
    }

    #[test]
    fn rejects_select_unknown_column() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);
        let binder = Binder::new(&catalog);
        let statement = parse_sql("select missing from users").unwrap();
        let err = unwrap_binder_err(binder.bind_statement(statement));

        assert!(matches!(err, BinderError::ColumnNotFound(column) if column == "missing"));
    }

    #[test]
    fn rejects_select_unknown_table() {
        let bpm = setup_bpm(3);
        let catalog = Catalog::new(&bpm);
        let binder = Binder::new(&catalog);
        let statement = parse_sql("select id from users").unwrap();
        let err = unwrap_binder_err(binder.bind_statement(statement));

        assert!(matches!(err, BinderError::Catalog(_)));
    }

    #[test]
    fn rejects_empty_select_projection() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);
        let binder = Binder::new(&catalog);
        let statement = Statement::Select(SelectStatement {
            table_name: "users".to_string(),
            projection: vec![],
            where_clause: None,
        });
        let err = unwrap_binder_err(binder.bind_statement(statement));

        assert!(matches!(err, BinderError::EmptySelectProjection));
    }

    #[test]
    fn rejects_duplicate_create_table_columns() {
        let bpm = setup_bpm(3);
        let catalog = Catalog::new(&bpm);
        let binder = Binder::new(&catalog);
        let statement = parse_sql("create table users (id integer, id integer)").unwrap();
        let err = unwrap_binder_err(binder.bind_statement(statement));

        assert!(matches!(err, BinderError::DuplicateColumn(column) if column == "id"));
    }

    #[test]
    fn rejects_insert_unknown_column() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);
        let binder = Binder::new(&catalog);
        let statement = parse_sql("insert into users (missing) values (1)").unwrap();
        let err = unwrap_binder_err(binder.bind_statement(statement));

        assert!(matches!(err, BinderError::ColumnNotFound(column) if column == "missing"));
    }

    #[test]
    fn rejects_insert_values_that_do_not_match_columns() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);
        let binder = Binder::new(&catalog);
        let statement = parse_sql("insert into users (id, name) values (1)").unwrap();
        let err = unwrap_binder_err(binder.bind_statement(statement));

        assert!(matches!(err, BinderError::InsertValuesDoesntMatchColumns));
    }

    #[test]
    fn rejects_column_refs_in_insert_values_without_scope() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);
        let binder = Binder::new(&catalog);
        let statement = parse_sql("insert into users (id) values (id)").unwrap();
        let err = unwrap_binder_err(binder.bind_statement(statement));

        assert!(
            matches!(err, BinderError::UnsupportedExpression(message) if message.contains("without table scope"))
        );
    }

    #[test]
    fn binds_insert_without_columns_using_schema_order() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);
        let binder = Binder::new(&catalog);
        let statement = parse_sql("insert into users values (1, 'alice')").unwrap();

        let bound = binder.bind_statement(statement).unwrap();
        let BoundStatement::Insert(insert) = bound else {
            panic!("expected insert statement");
        };

        expect![[r#"
            BoundInsert {
                table: BoundBaseTableRef {
                    table_name: "users",
                    table_oid: 0,
                    alias: None,
                    schema: Schema {
                        inlined_storage_size: 9,
                        columns: [
                            Column {
                                name: "id",
                                sql_type: Integer,
                                value_offset: 1,
                                size: Inline(
                                    4,
                                ),
                            },
                            Column {
                                name: "name",
                                sql_type: Varchar,
                                value_offset: 5,
                                size: Variable(
                                    32,
                                ),
                            },
                        ],
                        uninlined_columns: [
                            1,
                        ],
                    },
                },
                columns: [
                    Unqualified {
                        column: "id",
                    },
                    Unqualified {
                        column: "name",
                    },
                ],
                bound_exprs: BoundExpressionListRef {
                    identifier: "<unnamed>",
                    values: [
                        [
                            Literal(
                                Integer(
                                    1,
                                ),
                            ),
                            Literal(
                                Varchar(
                                    "alice",
                                ),
                            ),
                        ],
                    ],
                },
            }"#]]
        .assert_eq(&format!("{insert:#?}"));
    }

    #[test]
    fn rejects_duplicate_insert_columns() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);
        let binder = Binder::new(&catalog);
        let statement = parse_sql("insert into users (id, id) values (1, 2)").unwrap();
        let err = unwrap_binder_err(binder.bind_statement(statement));

        assert!(matches!(err, BinderError::DuplicateInsertColumns));
    }

    #[test]
    fn rejects_insert_without_columns_when_values_exceed_schema_columns() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);
        let binder = Binder::new(&catalog);
        let statement = parse_sql("insert into users values (1, 'alice', true)").unwrap();
        let err = unwrap_binder_err(binder.bind_statement(statement));

        assert!(matches!(err, BinderError::InsertValuesDoesntMatchColumns));
    }

    #[test]
    fn rejects_insert_without_columns_when_values_are_missing_schema_columns() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);
        let binder = Binder::new(&catalog);
        let statement = parse_sql("insert into users values (1)").unwrap();
        let err = unwrap_binder_err(binder.bind_statement(statement));

        assert!(matches!(err, BinderError::InsertValuesDoesntMatchColumns));
    }

    #[test]
    fn rejects_missing_primary_key_column() {
        let bpm = setup_bpm(3);
        let catalog = Catalog::new(&bpm);
        let binder = Binder::new(&catalog);
        let statement =
            parse_sql("create table users (id integer, primary key (missing))").unwrap();
        let err = unwrap_binder_err(binder.bind_statement(statement));

        assert!(
            matches!(err, BinderError::PrimaryKeyColumnNotFound(column) if column == "missing")
        );
    }

    #[test]
    fn rejects_duplicate_primary_key_column() {
        let bpm = setup_bpm(3);
        let catalog = Catalog::new(&bpm);
        let binder = Binder::new(&catalog);
        let statement = parse_sql("create table users (id integer, primary key (id, id))").unwrap();
        let err = unwrap_binder_err(binder.bind_statement(statement));

        assert!(matches!(err, BinderError::DuplicatePrimaryKeyColumn(column) if column == "id"));
    }
}
