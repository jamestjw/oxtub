use crate::{
    catalog::{column::Column, types::SqlType},
    query::expression::{BinaryOperator, UnaryOperator},
    types::value::Value,
};

#[derive(Debug, Clone, Copy)]
pub struct ExpressionType {
    pub sql_type: SqlType,
    pub varchar_size: Option<usize>,
}

impl ExpressionType {
    pub fn from_column(column: &Column) -> Self {
        Self {
            sql_type: column.sql_type(),
            varchar_size: if column.sql_type() == SqlType::Varchar {
                Some(column.declared_size())
            } else {
                None
            },
        }
    }

    pub fn from_value(value: &Value) -> Self {
        Self {
            sql_type: value.sql_type(),
            varchar_size: match value {
                Value::Varchar(s) => Some(s.len()),
                Value::Null(SqlType::Varchar) => Some(0),
                _ => None,
            },
        }
    }

    pub fn to_column(self, name: String) -> Column {
        match self.sql_type {
            SqlType::Varchar => {
                Column::new_variable(name, SqlType::Varchar, self.varchar_size.unwrap_or(0))
            }
            sql_type => Column::new_static(name, sql_type),
        }
    }
}

#[derive(Debug)]
pub struct PlannedExpression {
    pub return_type: ExpressionType,
    pub kind: PlannedExpressionKind,
}

#[derive(Debug)]
pub enum PlannedExpressionKind {
    ColumnValue(ColumnValueExpression),
    ConstantValue(ConstantValueExpression),
    BinaryOp(BinaryOpExpression),
    UnaryOp(UnaryOpExpression),
}

#[derive(Debug)]
pub struct ColumnValueExpression {
    // which child the column is from
    pub tuple_idx: usize,
    pub col_idx: usize,
}

#[derive(Debug)]
pub struct ConstantValueExpression {
    pub value: Value,
}

#[derive(Debug)]
pub struct BinaryOpExpression {
    pub left: Box<PlannedExpression>,
    pub op: BinaryOperator,
    pub right: Box<PlannedExpression>,
}

#[derive(Debug)]
pub struct UnaryOpExpression {
    pub op: UnaryOperator,
    pub expr: Box<PlannedExpression>,
}
