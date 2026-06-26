use crate::{
    catalog::column::Column,
    query::expression::{BinaryOperator, UnaryOperator},
    types::value::Value,
};

#[derive(Debug)]
pub struct PlannedExpression {
    pub return_type: Column,
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
