use crate::types::value::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    Column(ParsedColumnRef),
    Literal(Value),
    UnaryOp {
        op: UnaryOperator,
        expr: Box<Expression>,
    },
    BinaryOp {
        left: Box<Expression>,
        op: BinaryOperator,
        right: Box<Expression>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParsedColumnRef {
    pub qualifier: Option<ColumnQualifier>,
    pub column: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ColumnQualifier {
    Table { table: String },
    SchemaTable { schema: String, table: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOperator {
    Plus,
    Minus,
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOperator {
    Not,
    Neg,
    IsNull,
    IsNotNull,
}
