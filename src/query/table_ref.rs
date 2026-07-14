use crate::query::expression::Expression;

#[derive(Debug, Clone, PartialEq)]
pub enum TableRef {
    BaseTable {
        table_name: String,
        alias: Option<String>,
    },
    Join {
        left: Box<TableRef>,
        right: Box<TableRef>,
        join_type: JoinType,
        condition: Option<Expression>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinType {
    Inner,
    Left,
    Cross,
}
