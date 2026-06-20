use crate::types::value::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    Column(String),
    Literal(Value),
    BinaryOp {
        left: Box<Expression>,
        op: BinaryOp,
        right: Box<Expression>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    And,
    Or,
}
