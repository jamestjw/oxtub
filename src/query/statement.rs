use crate::{catalog::types::SqlType, query::expression::Expression};

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Select(SelectStatement),
    Insert(InsertStatement),
    CreateTable(CreateTableStatement),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectStatement {
    pub table_name: String,
    pub projection: Vec<SelectItem>,
    pub where_clause: Option<Expression>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SelectItem {
    Wildcard,
    Expression(Expression),
}

#[derive(Debug, Clone, PartialEq)]
pub struct InsertStatement {
    pub table_name: String,
    pub columns: Option<Vec<String>>,
    pub values: Vec<Vec<Expression>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateTableStatement {
    pub table_name: String,
    pub columns: Vec<CreateColumn>,
    pub primary_key: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateColumn {
    pub name: String,
    pub sql_type: SqlType,
    pub size: Option<usize>,
}
