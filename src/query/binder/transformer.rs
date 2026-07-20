use std::collections::HashSet;

use crate::{
    catalog::{column::Column, manager::Catalog, schema::Schema, types::SqlType},
    query::{
        binder::{
            error::BinderError,
            expression::{BoundExpression, ColumnRef, are_column_refs_unique},
            statement::{
                BoundCreateTable, BoundDelete, BoundInsert, BoundInsertSource, BoundSelect,
                BoundStatement, BoundUpdate,
            },
            table_ref::{BoundBaseTableRef, BoundExpressionListRef, BoundJoin, BoundTableRef},
        },
        expression::{ColumnQualifier, Expression, ParsedColumnRef},
        statement::{
            CreateColumn, CreateTableStatement, DeleteStatement, InsertSource, InsertStatement,
            SelectItem, SelectStatement, Statement, UpdateStatement,
        },
        table_ref::TableRef as ParsedTableRef,
    },
};

pub struct Binder<'catalog, 'bpm> {
    catalog: &'catalog Catalog<'bpm>,
}

struct BindContext<'a> {
    scope: Option<BindScope<'a>>,
}

enum BindScope<'a> {
    Tables(Vec<&'a BoundBaseTableRef>),
    ExprList(&'a BoundExpressionListRef),
}

impl<'a> BindContext<'a> {
    fn no_scope() -> Self {
        Self { scope: None }
    }

    fn base_table_scope(table: &'a BoundBaseTableRef) -> Self {
        Self {
            scope: Some(BindScope::Tables(vec![table])),
        }
    }

    fn table_ref_scope(table: &'a BoundTableRef) -> Self {
        match table {
            BoundTableRef::ExprList(expr_list) => Self {
                scope: Some(BindScope::ExprList(expr_list)),
            },
            BoundTableRef::BaseTable(_) | BoundTableRef::Join(_) => Self {
                scope: Some(BindScope::Tables(table.base_tables())),
            },
        }
    }

    fn table_ref_pair_scope(left: &'a BoundTableRef, right: &'a BoundTableRef) -> Self {
        let mut tables = left.base_tables();
        tables.extend(right.base_tables());

        Self {
            scope: Some(BindScope::Tables(tables)),
        }
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
            Statement::Update(update_statement) => self.bind_update(update_statement),
            Statement::Delete(delete_statement) => self.bind_delete(delete_statement),
            Statement::CreateTable(create_table_statement) => {
                self.bind_create_tbl(create_table_statement)
            }
        }
    }

    fn bind_update(&self, stmt: UpdateStatement) -> Result<BoundStatement, BinderError> {
        let table = self.bind_base_table_ref(stmt.table_name, None)?;
        let context = BindContext::base_table_scope(&table);
        let mut target_exprs = vec![];

        for (col_name, expr) in stmt.assignments {
            let col_ref = Self::resolve_column_ref_from_base_table_ref(&table, col_name)?;
            let bound_expr = self.bind_expression(expr, &context)?;

            target_exprs.push((col_ref, bound_expr));
        }

        let filter_expr = stmt
            .where_clause
            .map(|expr| self.bind_expression(expr, &context))
            .transpose()?;

        Ok(BoundStatement::Update(BoundUpdate {
            table,
            filter_expr,
            target_exprs,
        }))
    }

    fn bind_delete(&self, stmt: DeleteStatement) -> Result<BoundStatement, BinderError> {
        let table = self.bind_base_table_ref(stmt.table_name, None)?;
        let context = BindContext::base_table_scope(&table);
        let filter_expr = stmt
            .where_clause
            .map(|expr| self.bind_expression(expr, &context))
            .transpose()?;

        Ok(BoundStatement::Delete(BoundDelete { table, filter_expr }))
    }

    fn bind_table_ref(&self, table_ref: ParsedTableRef) -> Result<BoundTableRef, BinderError> {
        match table_ref {
            ParsedTableRef::BaseTable { table_name, alias } => Ok(BoundTableRef::BaseTable(
                self.bind_base_table_ref(table_name, alias)?,
            )),
            ParsedTableRef::Join {
                left,
                right,
                join_type,
                condition,
            } => {
                let left = self.bind_table_ref(*left)?;
                let right = self.bind_table_ref(*right)?;
                let context = BindContext::table_ref_pair_scope(&left, &right);
                let condition = match condition {
                    Some(expr) => Some(self.bind_expression(expr, &context)?),
                    None => None,
                };

                Ok(BoundTableRef::Join(BoundJoin::new(
                    left, right, join_type, condition,
                )))
            }
        }
    }

    fn bind_select(&self, stmt: SelectStatement) -> Result<BoundSelect, BinderError> {
        // TODO: can also select without a From clause
        let tbl_ref = self.bind_table_ref(stmt.table)?;
        let context = BindContext::table_ref_scope(&tbl_ref);
        let projection = self.bind_select_list(stmt.projection, &context)?;
        let where_ = stmt
            .where_clause
            .map(|expr| self.bind_expression(expr, &context))
            .transpose()?;

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
        match &context.scope {
            None => Err(BinderError::UnsupportedExpression(
                "select * without table scope".into(),
            )),
            Some(BindScope::Tables(tables)) => {
                let mut res = vec![];
                for table in tables {
                    let tbl_name = table.bound_tbl_name();
                    let cols = table.schema().columns();

                    for col in cols {
                        res.push(BoundExpression::Column(ColumnRef::TableQualified {
                            table: tbl_name.into(),
                            column: col.name().into(),
                        }));
                    }
                }

                Ok(res)
            }
            Some(BindScope::ExprList(_)) => panic!("select * should not use this table ref"),
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

        let primary_key_col_idxs = resolve_primary_key_col_idxs(&stmt.primary_key, &columns)?;

        Ok(BoundStatement::CreateTable(BoundCreateTable {
            name: stmt.table_name,
            columns,
            primary_key_col_idxs,
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
        let source = match stmt.source {
            InsertSource::Values(values) => {
                BoundInsertSource::Values(self.bind_values_list(num_columns, values)?)
            }
            InsertSource::Select(select) => BoundInsertSource::Select(self.bind_select(select)?),
        };

        Ok(BoundStatement::Insert(BoundInsert {
            table,
            columns,
            source,
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
        column: ParsedColumnRef,
        context: &BindContext<'_>,
    ) -> Result<ColumnRef, BinderError> {
        let ParsedColumnRef { qualifier, column } = column;

        match &context.scope {
            Some(BindScope::Tables(tables)) => {
                Self::resolve_column_ref_from_tables(&tables, qualifier, column)
            }
            Some(BindScope::ExprList(_)) => Err(BinderError::UnsupportedExpression(format!(
                "column reference `{column}` in expression list scope"
            ))),
            None => Err(BinderError::UnsupportedExpression(format!(
                "column reference `{column}` without table scope"
            ))),
        }
    }

    fn resolve_column_ref_from_tables(
        tables: &[&BoundBaseTableRef],
        qualifier: Option<ColumnQualifier>,
        column: String,
    ) -> Result<ColumnRef, BinderError> {
        match qualifier {
            None => Self::resolve_unqualified_column_ref_from_tables(tables, column),
            Some(ColumnQualifier::Table { table: qualifier }) => {
                let Some(table) = tables
                    .iter()
                    .copied()
                    .find(|table| table.matches_bound_tbl_name(&qualifier))
                else {
                    return Err(BinderError::MissingFromClauseEntry(qualifier));
                };

                Self::resolve_column_ref_from_base_table_ref(table, column)
            }
            Some(ColumnQualifier::SchemaTable { .. }) => todo!("schema not supported yet"),
        }
    }

    fn resolve_unqualified_column_ref_from_tables(
        tables: &[&BoundBaseTableRef],
        col_name: String,
    ) -> Result<ColumnRef, BinderError> {
        let mut resolved_column = None;

        for table in tables {
            if let Some(column_ref) = Self::resolve_column_ref_schema(table.schema(), &col_name)? {
                if resolved_column.is_some() {
                    return Err(BinderError::AmbiguousColumn(col_name));
                }

                let ColumnRef::Unqualified { column } = column_ref else {
                    unreachable!("schema column resolution should produce unqualified refs")
                };
                resolved_column = Some(ColumnRef::TableQualified {
                    table: table.bound_tbl_name().into(),
                    column,
                });
            }
        }

        resolved_column.ok_or(BinderError::ColumnNotFound(col_name))
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

fn resolve_primary_key_col_idxs(
    primary_key: &[String],
    columns: &[Column],
) -> Result<Vec<usize>, BinderError> {
    let mut seen_primary_key_cols = HashSet::new();
    let mut primary_key_col_idxs = Vec::with_capacity(primary_key.len());

    for primary_key_col in primary_key {
        let primary_key_col_key = primary_key_col.to_lowercase();

        if !seen_primary_key_cols.insert(primary_key_col_key.clone()) {
            return Err(BinderError::DuplicatePrimaryKeyColumn(
                primary_key_col.clone(),
            ));
        }

        let Some(col_idx) = columns
            .iter()
            .position(|column| column.name().to_lowercase() == primary_key_col_key)
        else {
            return Err(BinderError::PrimaryKeyColumnNotFound(
                primary_key_col.clone(),
            ));
        };

        primary_key_col_idxs.push(col_idx);
    }

    Ok(primary_key_col_idxs)
}

#[cfg(test)]
mod tests {
    use expect_test::expect;

    use crate::{
        catalog::{column::Column, schema::Schema},
        query::{binder::statement::BoundStatement, parser::parse_sql},
        testing::setup_bpm,
    };

    use super::*;

    fn unwrap_binder_err(result: Result<BoundStatement, BinderError>) -> BinderError {
        match result {
            Ok(_) => panic!("expected binder error"),
            Err(err) => err,
        }
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
                primary_key_col_idxs: [],
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
                primary_key_col_idxs: [
                    0,
                    1,
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
                source: Values(
                    BoundExpressionListRef {
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
                ),
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
    fn binds_join_condition_against_both_tables() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);
        create_table(&mut catalog, "orders");
        let binder = Binder::new(&catalog);
        let statement =
            parse_sql("select users.id from users join orders on users.id = orders.id").unwrap();

        let bound = binder.bind_statement(statement).unwrap();
        let BoundStatement::Select(select) = bound else {
            panic!("expected select statement");
        };

        expect![[r#"
            Join(
                BoundJoin {
                    left: BaseTable(
                        BoundBaseTableRef {
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
                    ),
                    right: BaseTable(
                        BoundBaseTableRef {
                            table_name: "orders",
                            table_oid: 1,
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
                    ),
                    join_type: Inner,
                    condition: Some(
                        BinaryOp {
                            left: Column(
                                TableQualified {
                                    table: "users",
                                    column: "id",
                                },
                            ),
                            op: Eq,
                            right: Column(
                                TableQualified {
                                    table: "orders",
                                    column: "id",
                                },
                            ),
                        },
                    ),
                },
            )"#]]
        .assert_eq(&format!("{:#?}", select.table));
    }

    #[test]
    fn binds_join_condition_against_aliased_tables() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);
        create_table(&mut catalog, "orders");
        let binder = Binder::new(&catalog);
        let statement = parse_sql("select u.id from users u join orders o on u.id = o.id").unwrap();

        let bound = binder.bind_statement(statement).unwrap();
        let BoundStatement::Select(select) = bound else {
            panic!("expected select statement");
        };

        expect![[r#"
            Join(
                BoundJoin {
                    left: BaseTable(
                        BoundBaseTableRef {
                            table_name: "users",
                            table_oid: 0,
                            alias: Some(
                                "u",
                            ),
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
                    ),
                    right: BaseTable(
                        BoundBaseTableRef {
                            table_name: "orders",
                            table_oid: 1,
                            alias: Some(
                                "o",
                            ),
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
                    ),
                    join_type: Inner,
                    condition: Some(
                        BinaryOp {
                            left: Column(
                                TableQualified {
                                    table: "u",
                                    column: "id",
                                },
                            ),
                            op: Eq,
                            right: Column(
                                TableQualified {
                                    table: "o",
                                    column: "id",
                                },
                            ),
                        },
                    ),
                },
            )"#]]
        .assert_eq(&format!("{:#?}", select.table));
    }

    #[test]
    fn binds_table_qualified_select_columns() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);
        let binder = Binder::new(&catalog);

        let statement = parse_sql("select users.id from users").unwrap();
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

        let statement = parse_sql("select u.id from users u").unwrap();
        let bound = binder.bind_statement(statement).unwrap();
        let BoundStatement::Select(select) = bound else {
            panic!("expected select statement");
        };

        assert_eq!(
            select.projection,
            vec![BoundExpression::Column(ColumnRef::TableQualified {
                table: "u".to_string(),
                column: "id".to_string(),
            })]
        );
    }

    #[test]
    fn rejects_table_qualified_columns_without_matching_table_scope() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);
        let binder = Binder::new(&catalog);

        let statement = parse_sql("select orders.id from users").unwrap();
        let err = unwrap_binder_err(binder.bind_statement(statement));
        assert!(matches!(err, BinderError::MissingFromClauseEntry(table) if table == "orders"));

        let statement = parse_sql("select users.id from users u").unwrap();
        let err = unwrap_binder_err(binder.bind_statement(statement));
        assert!(matches!(err, BinderError::MissingFromClauseEntry(table) if table == "users"));
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
    fn binds_update_with_assignments_and_where_clause() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);
        let binder = Binder::new(&catalog);
        let statement = parse_sql("update users set name = 'bob' where id = 1").unwrap();

        let bound = binder.bind_statement(statement).unwrap();
        let BoundStatement::Update(update) = bound else {
            panic!("expected update statement");
        };

        expect![[r#"
            BoundUpdate {
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
                filter_expr: Some(
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
                ),
                target_exprs: [
                    (
                        TableQualified {
                            table: "users",
                            column: "name",
                        },
                        Literal(
                            Varchar(
                                "bob",
                            ),
                        ),
                    ),
                ],
            }"#]]
        .assert_eq(&format!("{update:#?}"));
    }

    #[test]
    fn binds_update_without_where_clause() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);
        let binder = Binder::new(&catalog);
        let statement = parse_sql("update users set name = 'bob'").unwrap();

        let bound = binder.bind_statement(statement).unwrap();
        let BoundStatement::Update(update) = bound else {
            panic!("expected update statement");
        };

        assert!(update.filter_expr.is_none());
        assert_eq!(update.target_exprs.len(), 1);
    }

    #[test]
    fn binds_delete_with_where_clause() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);
        let binder = Binder::new(&catalog);
        let statement = parse_sql("delete from users where id = 1").unwrap();

        let bound = binder.bind_statement(statement).unwrap();
        let BoundStatement::Delete(delete) = bound else {
            panic!("expected delete statement");
        };

        expect![[r#"
            BoundDelete {
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
                filter_expr: Some(
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
                ),
            }"#]]
        .assert_eq(&format!("{delete:#?}"));
    }

    #[test]
    fn binds_delete_without_where_clause() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);
        let binder = Binder::new(&catalog);
        let statement = parse_sql("delete from users").unwrap();

        let bound = binder.bind_statement(statement).unwrap();
        let BoundStatement::Delete(delete) = bound else {
            panic!("expected delete statement");
        };

        assert!(delete.filter_expr.is_none());
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
    fn rejects_update_unknown_target_column() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);
        let binder = Binder::new(&catalog);
        let statement = parse_sql("update users set missing = 1").unwrap();
        let err = unwrap_binder_err(binder.bind_statement(statement));

        assert!(matches!(err, BinderError::ColumnNotFound(column) if column == "missing"));
    }

    #[test]
    fn rejects_update_unknown_filter_column() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);
        let binder = Binder::new(&catalog);
        let statement = parse_sql("update users set name = 'bob' where missing = 1").unwrap();
        let err = unwrap_binder_err(binder.bind_statement(statement));

        assert!(matches!(err, BinderError::ColumnNotFound(column) if column == "missing"));
    }

    #[test]
    fn rejects_delete_unknown_filter_column() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);
        let binder = Binder::new(&catalog);
        let statement = parse_sql("delete from users where missing = 1").unwrap();
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
            table: ParsedTableRef::BaseTable {
                table_name: "users".to_string(),
                alias: None,
            },
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
                source: Values(
                    BoundExpressionListRef {
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
                ),
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
