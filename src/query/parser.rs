use sqlparser::{
    ast::{
        AssignmentTarget, BinaryOperator as SqlBinaryOperator, CharacterLength, ColumnOption,
        DataType, Expr as SqlExpr, FromTable, Ident, IndexColumn, Join, JoinConstraint,
        JoinOperator, ObjectName, Query, Select, SelectItem as SqlSelectItem, SetExpr,
        Statement as SqlStatement, TableAlias, TableConstraint, TableFactor, TableObject,
        TableWithJoins, UnaryOperator as SqlUnaryOperator, Value as SqlValue,
    },
    dialect::PostgreSqlDialect,
    parser::Parser,
};

use crate::{
    catalog::types::SqlType,
    query::{
        error::QueryError,
        expression::{BinaryOperator, ColumnQualifier, Expression, ParsedColumnRef, UnaryOperator},
        statement::{
            CreateColumn, CreateTableStatement, DeleteStatement, InsertSource, InsertStatement,
            SelectItem, SelectStatement, Statement, UpdateStatement,
        },
        table_ref::{JoinType, TableRef},
    },
    types::value::Value,
};

pub fn parse_sql(sql: &str) -> Result<Statement, QueryError> {
    let dialect = PostgreSqlDialect {};
    let [statement]: [SqlStatement; 1] = Parser::parse_sql(&dialect, sql)?
        .try_into()
        .map_err(|_| QueryError::ExpectedSingleStatement)?;

    convert_statement(statement)
}

fn convert_statement(statement: SqlStatement) -> Result<Statement, QueryError> {
    match statement {
        SqlStatement::Query(query) => convert_query(*query),
        SqlStatement::Insert(insert) => convert_insert(insert),
        SqlStatement::Update(update) => convert_update(update),
        SqlStatement::Delete(delete) => convert_delete(delete),
        SqlStatement::CreateTable(create) => convert_create_table(create),
        _ => Err(QueryError::UnsupportedStatement(
            "only SELECT, INSERT, UPDATE, DELETE, and CREATE TABLE are supported",
        )),
    }
}

fn convert_query(query: Query) -> Result<Statement, QueryError> {
    Ok(Statement::Select(convert_query_to_select(query)?))
}

fn convert_query_to_select(query: Query) -> Result<SelectStatement, QueryError> {
    match *query.body {
        SetExpr::Select(select) => convert_select(*select),
        _ => Err(QueryError::UnsupportedQuery(
            "only SELECT queries are supported",
        )),
    }
}

fn convert_select(select: Select) -> Result<SelectStatement, QueryError> {
    let [table]: [TableWithJoins; 1] = select
        .from
        .try_into()
        .map_err(|_| QueryError::UnsupportedQuery("only one FROM table group is supported"))?;

    let projection = select
        .projection
        .into_iter()
        .map(convert_select_item)
        .collect::<Result<Vec<_>, _>>()?;
    let where_clause = select.selection.map(convert_expr).transpose()?;

    Ok(SelectStatement {
        table: convert_table_with_joins(table)?,
        projection,
        where_clause,
    })
}

fn convert_insert(insert: sqlparser::ast::Insert) -> Result<Statement, QueryError> {
    let source = insert
        .source
        .ok_or(QueryError::UnsupportedStatement("INSERT must use a source"))?;
    let source = match *source.body {
        SetExpr::Values(values) => InsertSource::Values(
            values
                .rows
                .into_iter()
                .map(|row| {
                    row.content
                        .into_iter()
                        .map(convert_expr)
                        .collect::<Result<Vec<_>, _>>()
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        SetExpr::Select(select) => InsertSource::Select(convert_select(*select)?),
        _ => {
            return Err(QueryError::UnsupportedStatement(
                "only VALUES and SELECT inserts are supported",
            ));
        }
    };

    let TableObject::TableName(table_name) = &insert.table else {
        return Err(QueryError::UnsupportedStatement(
            "INSERT target must be a table name",
        ));
    };

    Ok(Statement::Insert(InsertStatement {
        table_name: object_name_to_string(table_name),
        columns: non_empty_object_names(insert.columns),
        source,
    }))
}

fn convert_update(update: sqlparser::ast::Update) -> Result<Statement, QueryError> {
    if !update.optimizer_hints.is_empty() {
        return Err(QueryError::UnsupportedStatement(
            "UPDATE optimizer hints are not supported",
        ));
    }
    if update.from.is_some()
        || update.returning.is_some()
        || update.output.is_some()
        || update.or.is_some()
        || !update.order_by.is_empty()
        || update.limit.is_some()
    {
        return Err(QueryError::UnsupportedStatement(
            "only simple UPDATE statements are supported",
        ));
    }

    let table_name = table_name_from_single_table_with_joins(&update.table)?;
    let assignments = update
        .assignments
        .into_iter()
        .map(|assignment| match assignment.target {
            AssignmentTarget::ColumnName(column_name) => {
                let column_name = simple_object_name_to_string(&column_name).ok_or(
                    QueryError::UnsupportedStatement("qualified UPDATE targets are not supported"),
                )?;

                Ok((column_name, convert_expr(assignment.value)?))
            }
            _ => Err(QueryError::UnsupportedStatement(
                "UPDATE tuple assignments are not supported",
            )),
        })
        .collect::<Result<Vec<_>, QueryError>>()?;

    Ok(Statement::Update(UpdateStatement {
        table_name,
        assignments,
        where_clause: update.selection.map(convert_expr).transpose()?,
    }))
}

fn convert_delete(delete: sqlparser::ast::Delete) -> Result<Statement, QueryError> {
    if !delete.optimizer_hints.is_empty() {
        return Err(QueryError::UnsupportedStatement(
            "DELETE optimizer hints are not supported",
        ));
    }
    if !delete.tables.is_empty()
        || delete.using.is_some()
        || delete.returning.is_some()
        || delete.output.is_some()
        || !delete.order_by.is_empty()
        || delete.limit.is_some()
    {
        return Err(QueryError::UnsupportedStatement(
            "only simple DELETE statements are supported",
        ));
    }

    let tables = match &delete.from {
        FromTable::WithFromKeyword(tables) => tables,
        FromTable::WithoutKeyword(_) => {
            return Err(QueryError::UnsupportedStatement(
                "DELETE must use a FROM clause",
            ));
        }
    };
    let [table] = tables.as_slice() else {
        return Err(QueryError::UnsupportedStatement(
            "DELETE must target exactly one table",
        ));
    };

    Ok(Statement::Delete(DeleteStatement {
        table_name: table_name_from_single_table_with_joins(table)?,
        where_clause: delete.selection.map(convert_expr).transpose()?,
    }))
}

fn convert_create_table(create: sqlparser::ast::CreateTable) -> Result<Statement, QueryError> {
    let mut primary_key = Vec::new();
    let columns = create
        .columns
        .into_iter()
        .map(|column| {
            let column_name = ident_to_string(&column.name);
            if column_has_primary_key(&column)? {
                if !primary_key.is_empty() {
                    return Err(QueryError::UnsupportedStatement(
                        "multiple PRIMARY KEY declarations are not supported",
                    ));
                }
                primary_key.push(column_name.clone());
            }

            let (sql_type, size) = convert_data_type(column.data_type)?;
            Ok(CreateColumn {
                name: column_name,
                sql_type,
                size,
            })
        })
        .collect::<Result<Vec<_>, QueryError>>()?;

    for constraint in create.constraints {
        let TableConstraint::PrimaryKey(pk) = constraint else {
            return Err(QueryError::UnsupportedStatement(
                "only PRIMARY KEY constraints are supported in CREATE TABLE",
            ));
        };

        if !primary_key.is_empty() {
            return Err(QueryError::UnsupportedStatement(
                "multiple PRIMARY KEY declarations are not supported",
            ));
        }

        primary_key = pk
            .columns
            .into_iter()
            .map(convert_primary_key_column)
            .collect::<Result<Vec<_>, _>>()?;
    }

    Ok(Statement::CreateTable(CreateTableStatement {
        table_name: object_name_to_string(&create.name),
        columns,
        primary_key,
    }))
}

fn column_has_primary_key(column: &sqlparser::ast::ColumnDef) -> Result<bool, QueryError> {
    let mut has_primary_key = false;
    for option in &column.options {
        match option.option {
            ColumnOption::PrimaryKey(_) => {
                has_primary_key = true;
            }
            _ => {
                return Err(QueryError::UnsupportedStatement(
                    "only PRIMARY KEY column constraints are supported in CREATE TABLE",
                ));
            }
        }
    }

    Ok(has_primary_key)
}

fn convert_primary_key_column(column: IndexColumn) -> Result<String, QueryError> {
    match column.column.expr {
        SqlExpr::Identifier(ident) => Ok(ident_to_string(&ident)),
        _ => Err(QueryError::UnsupportedStatement(
            "PRIMARY KEY columns must be simple identifiers",
        )),
    }
}

fn convert_select_item(item: SqlSelectItem) -> Result<SelectItem, QueryError> {
    match item {
        SqlSelectItem::Wildcard(_) => Ok(SelectItem::Wildcard),
        SqlSelectItem::UnnamedExpr(expr) => Ok(SelectItem::Expression(convert_expr(expr)?)),
        _ => Err(QueryError::UnsupportedQuery(
            "only wildcard and unnamed select expressions are supported",
        )),
    }
}

fn convert_expr(expr: SqlExpr) -> Result<Expression, QueryError> {
    match expr {
        SqlExpr::Identifier(ident) => Ok(Expression::Column(ParsedColumnRef {
            qualifier: None,
            column: ident_to_string(&ident),
        })),
        SqlExpr::CompoundIdentifier(idents) => convert_compound_column_ref(idents),
        SqlExpr::Value(value) => convert_value(value.into()),
        SqlExpr::UnaryOp { op, expr } => Ok(Expression::UnaryOp {
            op: convert_unary_op(op)?,
            expr: Box::new(convert_expr(*expr)?),
        }),
        SqlExpr::BinaryOp { left, op, right } => Ok(Expression::BinaryOp {
            left: Box::new(convert_expr(*left)?),
            op: convert_binary_op(op)?,
            right: Box::new(convert_expr(*right)?),
        }),
        SqlExpr::IsNull(expr) => Ok(Expression::UnaryOp {
            op: UnaryOperator::IsNull,
            expr: Box::new(convert_expr(*expr)?),
        }),
        SqlExpr::IsNotNull(expr) => Ok(Expression::UnaryOp {
            op: UnaryOperator::IsNotNull,
            expr: Box::new(convert_expr(*expr)?),
        }),
        SqlExpr::Nested(expr) => convert_expr(*expr),
        _ => Err(QueryError::UnsupportedExpression),
    }
}

fn convert_compound_column_ref(idents: Vec<Ident>) -> Result<Expression, QueryError> {
    let column_ref = match idents.as_slice() {
        [table, column] => ParsedColumnRef {
            qualifier: Some(ColumnQualifier::Table {
                table: ident_to_string(table),
            }),
            column: ident_to_string(column),
        },
        [schema, table, column] => ParsedColumnRef {
            qualifier: Some(ColumnQualifier::SchemaTable {
                schema: ident_to_string(schema),
                table: ident_to_string(table),
            }),
            column: ident_to_string(column),
        },
        _ => return Err(QueryError::UnsupportedExpression),
    };

    Ok(Expression::Column(column_ref))
}

fn convert_table_with_joins(table: TableWithJoins) -> Result<TableRef, QueryError> {
    let mut table_ref = convert_table_factor(table.relation)?;

    for join in table.joins {
        let (join_type, right, condition) = convert_join(join)?;
        table_ref = TableRef::Join {
            left: Box::new(table_ref),
            right: Box::new(right),
            join_type,
            condition,
        };
    }

    Ok(table_ref)
}

fn convert_table_factor(factor: TableFactor) -> Result<TableRef, QueryError> {
    match factor {
        TableFactor::Table {
            name,
            alias,
            args,
            with_hints,
            version,
            with_ordinality,
            partitions,
            json_path,
            sample,
            index_hints,
        } => {
            if args.is_some()
                || !with_hints.is_empty()
                || version.is_some()
                || with_ordinality
                || !partitions.is_empty()
                || json_path.is_some()
                || sample.is_some()
                || !index_hints.is_empty()
            {
                return Err(QueryError::UnsupportedQuery(
                    "unsupported FROM table feature",
                ));
            }

            Ok(TableRef::BaseTable {
                table_name: object_name_to_string(&name),
                alias: convert_table_alias(alias)?,
            })
        }
        _ => Err(QueryError::UnsupportedQuery(
            "FROM source must be a base table",
        )),
    }
}

fn convert_join(join: Join) -> Result<(JoinType, TableRef, Option<Expression>), QueryError> {
    if join.global {
        return Err(QueryError::UnsupportedQuery("GLOBAL JOIN is not supported"));
    }

    let right = convert_table_factor(join.relation)?;
    let (join_type, condition) = match join.join_operator {
        JoinOperator::Join(constraint) | JoinOperator::Inner(constraint) => (
            JoinType::Inner,
            Some(convert_join_on_constraint(constraint)?),
        ),
        JoinOperator::Left(constraint) => (
            JoinType::Left,
            Some(convert_join_on_constraint(constraint)?),
        ),
        JoinOperator::CrossJoin(JoinConstraint::None) => (JoinType::Cross, None),
        _ => {
            return Err(QueryError::UnsupportedQuery(
                "only JOIN, INNER JOIN, LEFT JOIN, and CROSS JOIN are supported",
            ));
        }
    };

    Ok((join_type, right, condition))
}

fn convert_join_on_constraint(constraint: JoinConstraint) -> Result<Expression, QueryError> {
    match constraint {
        JoinConstraint::On(expr) => convert_expr(expr),
        _ => Err(QueryError::UnsupportedQuery(
            "JOIN and LEFT JOIN require an ON condition",
        )),
    }
}

fn convert_table_alias(alias: Option<TableAlias>) -> Result<Option<String>, QueryError> {
    match alias {
        Some(alias) => {
            if !alias.columns.is_empty() || alias.at.is_some() {
                return Err(QueryError::UnsupportedQuery(
                    "table alias columns are not supported",
                ));
            }

            Ok(Some(ident_to_string(&alias.name)))
        }
        None => Ok(None),
    }
}

fn convert_value(value: SqlValue) -> Result<Expression, QueryError> {
    match value {
        SqlValue::Boolean(b) => Ok(Expression::Literal(Value::Boolean(b))),
        SqlValue::Number(s, false) => s
            .parse::<i32>()
            .map(|i| Expression::Literal(Value::Integer(i)))
            .map_err(|_| QueryError::UnsupportedExpression),
        SqlValue::SingleQuotedString(s) => Ok(Expression::Literal(Value::Varchar(s))),
        SqlValue::Null => Ok(Expression::Literal(Value::Null(SqlType::Integer))),
        _ => Err(QueryError::UnsupportedExpression),
    }
}

fn convert_unary_op(op: SqlUnaryOperator) -> Result<UnaryOperator, QueryError> {
    match op {
        SqlUnaryOperator::Not => Ok(UnaryOperator::Not),
        SqlUnaryOperator::Minus => Ok(UnaryOperator::Neg),
        _ => Err(QueryError::UnsupportedExpression),
    }
}

fn convert_binary_op(op: SqlBinaryOperator) -> Result<BinaryOperator, QueryError> {
    match op {
        SqlBinaryOperator::Plus => Ok(BinaryOperator::Plus),
        SqlBinaryOperator::Minus => Ok(BinaryOperator::Minus),
        SqlBinaryOperator::Eq => Ok(BinaryOperator::Eq),
        SqlBinaryOperator::NotEq => Ok(BinaryOperator::NotEq),
        SqlBinaryOperator::Lt => Ok(BinaryOperator::Lt),
        SqlBinaryOperator::LtEq => Ok(BinaryOperator::LtEq),
        SqlBinaryOperator::Gt => Ok(BinaryOperator::Gt),
        SqlBinaryOperator::GtEq => Ok(BinaryOperator::GtEq),
        SqlBinaryOperator::And => Ok(BinaryOperator::And),
        SqlBinaryOperator::Or => Ok(BinaryOperator::Or),
        _ => Err(QueryError::UnsupportedExpression),
    }
}

fn convert_data_type(data_type: DataType) -> Result<(SqlType, Option<usize>), QueryError> {
    match data_type {
        DataType::Boolean => Ok((SqlType::Boolean, None)),
        DataType::SmallInt(_) => Ok((SqlType::SmallInt, None)),
        DataType::Int(_) | DataType::Integer(_) => Ok((SqlType::Integer, None)),
        DataType::BigInt(_) => Ok((SqlType::BigInt, None)),
        DataType::Double(_) => Ok((SqlType::Decimal, None)),
        DataType::Varchar(Some(CharacterLength::IntegerLength { length, .. })) => {
            Ok((SqlType::Varchar, Some(length as usize)))
        }
        DataType::Varchar(None) => Err(QueryError::VarcharMissingSize),
        unsupported => Err(QueryError::UnsupportedDataType(unsupported.to_string())),
    }
}

fn non_empty_object_names(names: Vec<ObjectName>) -> Option<Vec<String>> {
    if names.is_empty() {
        None
    } else {
        Some(names.iter().map(object_name_to_string).collect())
    }
}

fn ident_to_string(ident: &Ident) -> String {
    ident.value.clone()
}

fn object_name_to_string(name: &ObjectName) -> String {
    name.to_string()
}

fn simple_object_name_to_string(name: &ObjectName) -> Option<String> {
    let [part] = name.0.as_slice() else {
        return None;
    };

    part.as_ident().map(ident_to_string)
}

fn table_name_from_single_table_with_joins(table: &TableWithJoins) -> Result<String, QueryError> {
    if !table.joins.is_empty() {
        return Err(QueryError::UnsupportedStatement(
            "joins are not supported yet",
        ));
    }

    let TableFactor::Table { name, .. } = &table.relation else {
        return Err(QueryError::UnsupportedStatement(
            "statement target must be a base table",
        ));
    };

    Ok(object_name_to_string(name))
}

#[cfg(test)]
mod tests {
    use expect_test::expect;

    use crate::query::error::QueryError;

    use super::*;

    #[test]
    fn parses_select_wildcard() {
        let statement = parse_sql("select * from users").unwrap();

        expect![[r#"
            Select(
                SelectStatement {
                    table: BaseTable {
                        table_name: "users",
                        alias: None,
                    },
                    projection: [
                        Wildcard,
                    ],
                    where_clause: None,
                },
            )"#]]
        .assert_eq(&format!("{statement:#?}"));
    }

    #[test]
    fn parses_select_projection() {
        let statement = parse_sql("select id, name from users").unwrap();

        expect![[r#"
            Select(
                SelectStatement {
                    table: BaseTable {
                        table_name: "users",
                        alias: None,
                    },
                    projection: [
                        Expression(
                            Column(
                                ParsedColumnRef {
                                    qualifier: None,
                                    column: "id",
                                },
                            ),
                        ),
                        Expression(
                            Column(
                                ParsedColumnRef {
                                    qualifier: None,
                                    column: "name",
                                },
                            ),
                        ),
                    ],
                    where_clause: None,
                },
            )"#]]
        .assert_eq(&format!("{statement:#?}"));
    }

    #[test]
    fn parses_select_joins() {
        let statement =
            parse_sql("select * from users join orders on users.id = orders.user_id").unwrap();

        expect![[r#"
            Select(
                SelectStatement {
                    table: Join {
                        left: BaseTable {
                            table_name: "users",
                            alias: None,
                        },
                        right: BaseTable {
                            table_name: "orders",
                            alias: None,
                        },
                        join_type: Inner,
                        condition: Some(
                            BinaryOp {
                                left: Column(
                                    ParsedColumnRef {
                                        qualifier: Some(
                                            Table {
                                                table: "users",
                                            },
                                        ),
                                        column: "id",
                                    },
                                ),
                                op: Eq,
                                right: Column(
                                    ParsedColumnRef {
                                        qualifier: Some(
                                            Table {
                                                table: "orders",
                                            },
                                        ),
                                        column: "user_id",
                                    },
                                ),
                            },
                        ),
                    },
                    projection: [
                        Wildcard,
                    ],
                    where_clause: None,
                },
            )"#]]
        .assert_eq(&format!("{statement:#?}"));

        let statement =
            parse_sql("select * from users inner join orders on users.id = orders.user_id")
                .unwrap();

        expect![[r#"Select(SelectStatement { table: Join { left: BaseTable { table_name: "users", alias: None }, right: BaseTable { table_name: "orders", alias: None }, join_type: Inner, condition: Some(BinaryOp { left: Column(ParsedColumnRef { qualifier: Some(Table { table: "users" }), column: "id" }), op: Eq, right: Column(ParsedColumnRef { qualifier: Some(Table { table: "orders" }), column: "user_id" }) }) }, projection: [Wildcard], where_clause: None })"#]]
            .assert_eq(&format!("{statement:?}"));

        let statement =
            parse_sql("select * from users left join orders on users.id = orders.user_id").unwrap();

        expect![[r#"Select(SelectStatement { table: Join { left: BaseTable { table_name: "users", alias: None }, right: BaseTable { table_name: "orders", alias: None }, join_type: Left, condition: Some(BinaryOp { left: Column(ParsedColumnRef { qualifier: Some(Table { table: "users" }), column: "id" }), op: Eq, right: Column(ParsedColumnRef { qualifier: Some(Table { table: "orders" }), column: "user_id" }) }) }, projection: [Wildcard], where_clause: None })"#]]
            .assert_eq(&format!("{statement:?}"));

        let statement = parse_sql("select * from users cross join orders").unwrap();

        expect![[r#"Select(SelectStatement { table: Join { left: BaseTable { table_name: "users", alias: None }, right: BaseTable { table_name: "orders", alias: None }, join_type: Cross, condition: None }, projection: [Wildcard], where_clause: None })"#]]
            .assert_eq(&format!("{statement:?}"));
    }

    #[test]
    fn parses_select_join_aliases() {
        let statement =
            parse_sql("select * from users u join orders o on u.id = o.user_id").unwrap();

        expect![[r#"Select(SelectStatement { table: Join { left: BaseTable { table_name: "users", alias: Some("u") }, right: BaseTable { table_name: "orders", alias: Some("o") }, join_type: Inner, condition: Some(BinaryOp { left: Column(ParsedColumnRef { qualifier: Some(Table { table: "u" }), column: "id" }), op: Eq, right: Column(ParsedColumnRef { qualifier: Some(Table { table: "o" }), column: "user_id" }) }) }, projection: [Wildcard], where_clause: None })"#]]
            .assert_eq(&format!("{statement:?}"));
    }

    #[test]
    fn rejects_unsupported_select_joins() {
        for sql in [
            "select * from users right join orders on users.id = orders.user_id",
            "select * from users full join orders on users.id = orders.user_id",
            "select * from users natural join orders",
            "select * from users join orders using (id)",
            "select * from users, orders",
        ] {
            let err = parse_sql(sql).unwrap_err();
            assert!(matches!(err, QueryError::UnsupportedQuery(_)));
        }
    }

    #[test]
    fn parses_select_projection_unary_operators() {
        let statement =
            parse_sql("select not active, -score, name is null, name is not null from users")
                .unwrap();

        expect![[r#"Select(SelectStatement { table: BaseTable { table_name: "users", alias: None }, projection: [Expression(UnaryOp { op: Not, expr: Column(ParsedColumnRef { qualifier: None, column: "active" }) }), Expression(UnaryOp { op: Neg, expr: Column(ParsedColumnRef { qualifier: None, column: "score" }) }), Expression(UnaryOp { op: IsNull, expr: Column(ParsedColumnRef { qualifier: None, column: "name" }) }), Expression(UnaryOp { op: IsNotNull, expr: Column(ParsedColumnRef { qualifier: None, column: "name" }) })], where_clause: None })"#]]
            .assert_eq(&format!("{statement:?}"));
    }

    #[test]
    fn parses_select_projection_arithmetic_operators() {
        let statement = parse_sql("select score + 1, score - 1, -score + 1 from users").unwrap();

        expect![[r#"Select(SelectStatement { table: BaseTable { table_name: "users", alias: None }, projection: [Expression(BinaryOp { left: Column(ParsedColumnRef { qualifier: None, column: "score" }), op: Plus, right: Literal(Integer(1)) }), Expression(BinaryOp { left: Column(ParsedColumnRef { qualifier: None, column: "score" }), op: Minus, right: Literal(Integer(1)) }), Expression(BinaryOp { left: UnaryOp { op: Neg, expr: Column(ParsedColumnRef { qualifier: None, column: "score" }) }, op: Plus, right: Literal(Integer(1)) })], where_clause: None })"#]]
            .assert_eq(&format!("{statement:?}"));
    }

    #[test]
    fn parses_select_where_equality() {
        let statement = parse_sql("select id from users where id = 1").unwrap();

        expect![[r#"
            Select(
                SelectStatement {
                    table: BaseTable {
                        table_name: "users",
                        alias: None,
                    },
                    projection: [
                        Expression(
                            Column(
                                ParsedColumnRef {
                                    qualifier: None,
                                    column: "id",
                                },
                            ),
                        ),
                    ],
                    where_clause: Some(
                        BinaryOp {
                            left: Column(
                                ParsedColumnRef {
                                    qualifier: None,
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
                },
            )"#]]
        .assert_eq(&format!("{statement:#?}"));
    }

    #[test]
    fn parses_select_where_unary_operators() {
        let statement = parse_sql("select id from users where not active").unwrap();

        expect![[r#"Select(SelectStatement { table: BaseTable { table_name: "users", alias: None }, projection: [Expression(Column(ParsedColumnRef { qualifier: None, column: "id" }))], where_clause: Some(UnaryOp { op: Not, expr: Column(ParsedColumnRef { qualifier: None, column: "active" }) }) })"#]]
            .assert_eq(&format!("{statement:?}"));

        let statement = parse_sql("select id from users where -score = 1").unwrap();

        expect![[r#"Select(SelectStatement { table: BaseTable { table_name: "users", alias: None }, projection: [Expression(Column(ParsedColumnRef { qualifier: None, column: "id" }))], where_clause: Some(BinaryOp { left: UnaryOp { op: Neg, expr: Column(ParsedColumnRef { qualifier: None, column: "score" }) }, op: Eq, right: Literal(Integer(1)) }) })"#]]
            .assert_eq(&format!("{statement:?}"));

        let statement = parse_sql("select id from users where name is null").unwrap();

        expect![[r#"Select(SelectStatement { table: BaseTable { table_name: "users", alias: None }, projection: [Expression(Column(ParsedColumnRef { qualifier: None, column: "id" }))], where_clause: Some(UnaryOp { op: IsNull, expr: Column(ParsedColumnRef { qualifier: None, column: "name" }) }) })"#]]
            .assert_eq(&format!("{statement:?}"));

        let statement = parse_sql("select id from users where name is not null").unwrap();

        expect![[r#"Select(SelectStatement { table: BaseTable { table_name: "users", alias: None }, projection: [Expression(Column(ParsedColumnRef { qualifier: None, column: "id" }))], where_clause: Some(UnaryOp { op: IsNotNull, expr: Column(ParsedColumnRef { qualifier: None, column: "name" }) }) })"#]]
            .assert_eq(&format!("{statement:?}"));

        let statement = parse_sql("select id from users where not (name is null)").unwrap();

        expect![[r#"Select(SelectStatement { table: BaseTable { table_name: "users", alias: None }, projection: [Expression(Column(ParsedColumnRef { qualifier: None, column: "id" }))], where_clause: Some(UnaryOp { op: Not, expr: UnaryOp { op: IsNull, expr: Column(ParsedColumnRef { qualifier: None, column: "name" }) } }) })"#]]
            .assert_eq(&format!("{statement:?}"));
    }

    #[test]
    fn parses_insert_values_without_columns() {
        let statement = parse_sql("insert into users values (1, 'alice')").unwrap();

        expect![[r#"
            Insert(
                InsertStatement {
                    table_name: "users",
                    columns: None,
                    source: Values(
                        [
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
                    ),
                },
            )"#]]
        .assert_eq(&format!("{statement:#?}"));

        let statement = parse_sql("insert into users values (1, 'alice'), (2, 'bob')").unwrap();

        expect![[r#"
            Insert(
                InsertStatement {
                    table_name: "users",
                    columns: None,
                    source: Values(
                        [
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
                    ),
                },
            )"#]]
        .assert_eq(&format!("{statement:#?}"));
    }

    #[test]
    fn parses_insert_columns_and_values() {
        let statement =
            parse_sql("insert into users (id, name) values (1, 'alice'), (2, 'bob')").unwrap();

        expect![[r#"
            Insert(
                InsertStatement {
                    table_name: "users",
                    columns: Some(
                        [
                            "id",
                            "name",
                        ],
                    ),
                    source: Values(
                        [
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
                    ),
                },
            )"#]]
        .assert_eq(&format!("{statement:#?}"));
    }

    #[test]
    fn parses_insert_select() {
        let statement = parse_sql("insert into t2 select * from t1").unwrap();

        expect![[r#"
            Insert(
                InsertStatement {
                    table_name: "t2",
                    columns: None,
                    source: Select(
                        SelectStatement {
                            table: BaseTable {
                                table_name: "t1",
                                alias: None,
                            },
                            projection: [
                                Wildcard,
                            ],
                            where_clause: None,
                        },
                    ),
                },
            )"#]]
        .assert_eq(&format!("{statement:#?}"));
    }

    #[test]
    fn parses_update_statement() {
        let statement =
            parse_sql("update users set name = 'bob', active = true where id = 1").unwrap();

        expect![[r#"
            Update(
                UpdateStatement {
                    table_name: "users",
                    assignments: [
                        (
                            "name",
                            Literal(
                                Varchar(
                                    "bob",
                                ),
                            ),
                        ),
                        (
                            "active",
                            Literal(
                                Boolean(
                                    true,
                                ),
                            ),
                        ),
                    ],
                    where_clause: Some(
                        BinaryOp {
                            left: Column(
                                ParsedColumnRef {
                                    qualifier: None,
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
                },
            )"#]]
        .assert_eq(&format!("{statement:#?}"));
    }

    #[test]
    fn parses_delete_statement() {
        let statement = parse_sql("delete from users where id = 1").unwrap();

        expect![[r#"
            Delete(
                DeleteStatement {
                    table_name: "users",
                    where_clause: Some(
                        BinaryOp {
                            left: Column(
                                ParsedColumnRef {
                                    qualifier: None,
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
                },
            )"#]]
        .assert_eq(&format!("{statement:#?}"));
    }

    #[test]
    fn rejects_qualified_update_targets() {
        let err = parse_sql("update users set users.name = 'bob'").unwrap_err();

        assert!(matches!(err, QueryError::UnsupportedStatement(_)));
    }

    #[test]
    fn parses_create_table_with_varchar_size() {
        let statement = parse_sql("create table users (id integer, name varchar(32))").unwrap();

        expect![[r#"
            CreateTable(
                CreateTableStatement {
                    table_name: "users",
                    columns: [
                        CreateColumn {
                            name: "id",
                            sql_type: Integer,
                            size: None,
                        },
                        CreateColumn {
                            name: "name",
                            sql_type: Varchar,
                            size: Some(
                                32,
                            ),
                        },
                    ],
                    primary_key: [],
                },
            )"#]]
        .assert_eq(&format!("{statement:#?}"));
    }

    #[test]
    fn parses_create_table_with_column_primary_key() {
        let statement =
            parse_sql("create table users (id integer primary key, name varchar(32))").unwrap();

        expect![[r#"
            CreateTable(
                CreateTableStatement {
                    table_name: "users",
                    columns: [
                        CreateColumn {
                            name: "id",
                            sql_type: Integer,
                            size: None,
                        },
                        CreateColumn {
                            name: "name",
                            sql_type: Varchar,
                            size: Some(
                                32,
                            ),
                        },
                    ],
                    primary_key: [
                        "id",
                    ],
                },
            )"#]]
        .assert_eq(&format!("{statement:#?}"));
    }

    #[test]
    fn parses_create_table_with_table_primary_key() {
        let statement =
            parse_sql("create table users (id integer, name varchar(32), primary key (id, name))")
                .unwrap();

        expect![[r#"
            CreateTable(
                CreateTableStatement {
                    table_name: "users",
                    columns: [
                        CreateColumn {
                            name: "id",
                            sql_type: Integer,
                            size: None,
                        },
                        CreateColumn {
                            name: "name",
                            sql_type: Varchar,
                            size: Some(
                                32,
                            ),
                        },
                    ],
                    primary_key: [
                        "id",
                        "name",
                    ],
                },
            )"#]]
        .assert_eq(&format!("{statement:#?}"));
    }

    #[test]
    fn rejects_non_primary_key_create_table_constraints() {
        let err = parse_sql("create table users (id integer not null)").unwrap_err();
        assert!(matches!(err, QueryError::UnsupportedStatement(_)));

        let err = parse_sql("create table users (id integer, unique (id))").unwrap_err();
        assert!(matches!(err, QueryError::UnsupportedStatement(_)));
    }

    #[test]
    fn rejects_multiple_primary_key_declarations() {
        let err =
            parse_sql("create table users (id integer primary key, primary key (id))").unwrap_err();

        assert!(matches!(err, QueryError::UnsupportedStatement(_)));
    }

    #[test]
    fn rejects_create_table_with_bare_varchar() {
        let err = parse_sql("create table users (name varchar)").unwrap_err();

        assert!(matches!(err, QueryError::VarcharMissingSize));
    }

    #[test]
    fn rejects_float_and_real_data_types() {
        for sql_type in ["FLOAT", "REAL"] {
            let err = parse_sql(&format!("create table users (score {sql_type})")).unwrap_err();

            assert!(
                matches!(err, QueryError::UnsupportedDataType(data_type) if data_type.to_uppercase() == sql_type)
            );
        }
    }

    #[test]
    fn rejects_multiple_statements() {
        let err = parse_sql("select * from users; select * from orders;").unwrap_err();

        assert!(matches!(err, QueryError::ExpectedSingleStatement));
    }
}
