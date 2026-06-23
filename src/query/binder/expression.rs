use std::collections::HashSet;

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

pub fn are_column_refs_unique(column_refs: &[ColumnRef]) -> bool {
    let mut seen = HashSet::new();

    for column_ref in column_refs {
        let stringified_col = match column_ref {
            ColumnRef::Unqualified { column } => column.clone(),
            ColumnRef::TableQualified { table, column } => format!("{table}.{column}"),
        };
        if !seen.insert(stringified_col.to_lowercase()) {
            return false;
        }
    }

    true
}
