use crate::{
    query::expression::{BinaryOperator, UnaryOperator},
    types::value::Value,
};

#[derive(Debug, PartialEq)]
pub enum ColumnRef {
    Unqualified { column: String },
    TableQualified { table: String, column: String },
}

#[derive(Debug, PartialEq)]
pub enum BoundExpression {
    Literal(Value),
    Column(ColumnRef),
    Star,
    BinaryOp {
        left: Box<BoundExpression>,
        op: BinaryOperator,
        right: Box<BoundExpression>,
    },
    UnaryOp {
        expr: Box<BoundExpression>,
        op: UnaryOperator,
    },
}
