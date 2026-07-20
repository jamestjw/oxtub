use std::collections::HashSet;

use crate::{
    query::expression::{BinaryOperator, UnaryOperator},
    types::value::Value,
};

#[derive(Debug, Clone, PartialEq)]
pub enum ColumnRef {
    Unqualified {
        column: String,
    },
    TableQualified {
        table: String,
        column: String,
    },
    SchemaTableQualified {
        schema: String,
        table: String,
        column: String,
    },
}

impl ColumnRef {
    pub fn to_str(&self) -> String {
        match self {
            ColumnRef::Unqualified { column } => column.clone(),
            ColumnRef::TableQualified { table, column } => format!("{table}.{column}"),
            ColumnRef::SchemaTableQualified {
                schema,
                table,
                column,
            } => format!("{schema}.{table}.{column}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BoundExpression {
    Literal(Value),
    Column(ColumnRef),
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
        let stringified_col = column_ref.to_str();
        if !seen.insert(stringified_col.to_lowercase()) {
            return false;
        }
    }

    true
}
