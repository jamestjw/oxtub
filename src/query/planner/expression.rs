use crate::{
    catalog::{column::Column, types::SqlType},
    types::value::Value,
};

#[derive(Debug, Clone, Copy)]
pub struct ExpressionType {
    pub sql_type: SqlType,
    pub varchar_size: Option<usize>,
}

impl ExpressionType {
    pub fn new_bool() -> Self {
        Self {
            sql_type: SqlType::Boolean,
            varchar_size: None,
        }
    }

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
    Comparison(ComparisonExpression),
    Arithmetic(ArithmeticExpression),
    Logic(LogicExpression),
    Not(NotExpression),
    Negate(NegateExpression),
    NullCheck(NullCheckExpression),
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
pub enum ComparisonType {
    Eq,
    NotEq,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
}

#[derive(Debug)]
pub enum ArithmeticType {
    Plus,
    Minus,
}

#[derive(Debug)]
pub enum LogicType {
    And,
    Or,
}

#[derive(Debug)]
pub enum NullCheckType {
    IsNull,
    IsNotNull,
}

#[derive(Debug)]
pub struct ComparisonExpression {
    pub left: Box<PlannedExpression>,
    pub comparison_type: ComparisonType,
    pub right: Box<PlannedExpression>,
}

#[derive(Debug)]
pub struct ArithmeticExpression {
    pub left: Box<PlannedExpression>,
    pub arithmetic_type: ArithmeticType,
    pub right: Box<PlannedExpression>,
}

#[derive(Debug)]
pub struct LogicExpression {
    pub left: Box<PlannedExpression>,
    pub logic_type: LogicType,
    pub right: Box<PlannedExpression>,
}

#[derive(Debug)]
pub struct NotExpression {
    pub expr: Box<PlannedExpression>,
}

#[derive(Debug)]
pub struct NegateExpression {
    pub expr: Box<PlannedExpression>,
}

#[derive(Debug)]
pub struct NullCheckExpression {
    pub expr: Box<PlannedExpression>,
    pub null_check_type: NullCheckType,
}
