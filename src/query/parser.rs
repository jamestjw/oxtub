use sqlparser::{
    ast::{
        BinaryOperator, CharacterLength, DataType, Expr as SqlExpr, Ident, ObjectName, Query,
        Select, SelectItem as SqlSelectItem, SetExpr, Statement as SqlStatement, TableFactor,
        TableObject, Value as SqlValue,
    },
    dialect::PostgreSqlDialect,
    parser::Parser,
};

use crate::{
    catalog::types::SqlType,
    query::{
        error::QueryError,
        expression::{BinaryOp, Expression},
        statement::{
            CreateColumn, CreateTableStatement, InsertStatement, SelectItem, SelectStatement,
            Statement,
        },
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
        SqlStatement::CreateTable(create) => convert_create_table(create),
        _ => Err(QueryError::UnsupportedStatement(
            "only SELECT, INSERT, and CREATE TABLE are supported",
        )),
    }
}

fn convert_query(query: Query) -> Result<Statement, QueryError> {
    match *query.body {
        SetExpr::Select(select) => convert_select(*select),
        _ => Err(QueryError::UnsupportedQuery(
            "only SELECT queries are supported",
        )),
    }
}

fn convert_select(select: Select) -> Result<Statement, QueryError> {
    let [table] = select.from.as_slice() else {
        return Err(QueryError::UnsupportedQuery(
            "only one FROM table group is supported",
        ));
    };

    if !table.joins.is_empty() {
        // TODO: support joins within a single TableWithJoins. We can still keep
        // multiple comma-separated TableWithJoins unsupported for now.
        return Err(QueryError::UnsupportedQuery("joins are not supported yet"));
    }

    let TableFactor::Table { name, .. } = &table.relation else {
        return Err(QueryError::UnsupportedQuery(
            "FROM source must be a base table",
        ));
    };

    let projection = select
        .projection
        .into_iter()
        .map(convert_select_item)
        .collect::<Result<Vec<_>, _>>()?;
    let where_clause = select.selection.map(convert_expr).transpose()?;

    Ok(Statement::Select(SelectStatement {
        table_name: object_name_to_string(name),
        projection,
        where_clause,
    }))
}

fn convert_insert(insert: sqlparser::ast::Insert) -> Result<Statement, QueryError> {
    let source = insert.source.ok_or(QueryError::UnsupportedStatement(
        "INSERT must use a VALUES source",
    ))?;
    let rows = match *source.body {
        SetExpr::Values(values) => values
            .rows
            .into_iter()
            .map(|row| {
                row.content
                    .into_iter()
                    .map(convert_expr)
                    .collect::<Result<Vec<_>, _>>()
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => {
            return Err(QueryError::UnsupportedStatement(
                "only VALUES inserts are supported",
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
        values: rows,
    }))
}

fn convert_create_table(create: sqlparser::ast::CreateTable) -> Result<Statement, QueryError> {
    let columns = create
        .columns
        .into_iter()
        .map(|column| {
            let (sql_type, size) = convert_data_type(column.data_type)?;
            Ok(CreateColumn {
                name: ident_to_string(&column.name),
                sql_type,
                size,
            })
        })
        .collect::<Result<Vec<_>, QueryError>>()?;

    Ok(Statement::CreateTable(CreateTableStatement {
        table_name: object_name_to_string(&create.name),
        columns,
    }))
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
        SqlExpr::Identifier(ident) => Ok(Expression::Column(ident_to_string(&ident))),
        SqlExpr::Value(value) => convert_value(value.into()),
        SqlExpr::BinaryOp { left, op, right } => Ok(Expression::BinaryOp {
            left: Box::new(convert_expr(*left)?),
            op: convert_binary_op(op)?,
            right: Box::new(convert_expr(*right)?),
        }),
        _ => Err(QueryError::UnsupportedExpression),
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

fn convert_binary_op(op: BinaryOperator) -> Result<BinaryOp, QueryError> {
    match op {
        BinaryOperator::Eq => Ok(BinaryOp::Eq),
        BinaryOperator::NotEq => Ok(BinaryOp::NotEq),
        BinaryOperator::Lt => Ok(BinaryOp::Lt),
        BinaryOperator::LtEq => Ok(BinaryOp::LtEq),
        BinaryOperator::Gt => Ok(BinaryOp::Gt),
        BinaryOperator::GtEq => Ok(BinaryOp::GtEq),
        BinaryOperator::And => Ok(BinaryOp::And),
        BinaryOperator::Or => Ok(BinaryOp::Or),
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
                    table_name: "users",
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
                    table_name: "users",
                    projection: [
                        Expression(
                            Column(
                                "id",
                            ),
                        ),
                        Expression(
                            Column(
                                "name",
                            ),
                        ),
                    ],
                    where_clause: None,
                },
            )"#]]
        .assert_eq(&format!("{statement:#?}"));
    }

    #[test]
    fn parses_select_where_equality() {
        let statement = parse_sql("select id from users where id = 1").unwrap();

        expect![[r#"
            Select(
                SelectStatement {
                    table_name: "users",
                    projection: [
                        Expression(
                            Column(
                                "id",
                            ),
                        ),
                    ],
                    where_clause: Some(
                        BinaryOp {
                            left: Column(
                                "id",
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
    fn parses_insert_values_without_columns() {
        let statement = parse_sql("insert into users values (1, 'alice')").unwrap();

        expect![[r#"
            Insert(
                InsertStatement {
                    table_name: "users",
                    columns: None,
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
            )"#]]
        .assert_eq(&format!("{statement:#?}"));

        let statement = parse_sql("insert into users values (1, 'alice'), (2, 'bob')").unwrap();

        expect![[r#"
            Insert(
                InsertStatement {
                    table_name: "users",
                    columns: None,
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
            )"#]]
        .assert_eq(&format!("{statement:#?}"));
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
                },
            )"#]]
        .assert_eq(&format!("{statement:#?}"));
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
