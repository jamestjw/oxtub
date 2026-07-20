use crate::{
    catalog::types::SqlType,
    query::{expression::Expression, table_ref::TableRef},
};

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Select(SelectStatement),
    Insert(InsertStatement),
    Update(UpdateStatement),
    Delete(DeleteStatement),
    Explain(ExplainStatement),
    CreateTable(CreateTableStatement),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExplainStatement {
    pub raw: bool,
    pub statement: Box<Statement>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectStatement {
    pub table: TableRef,
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
    pub source: InsertSource,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InsertSource {
    Values(Vec<Vec<Expression>>),
    Select(SelectStatement),
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateStatement {
    pub table_name: String,
    pub assignments: Vec<(String, Expression)>,
    pub where_clause: Option<Expression>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeleteStatement {
    pub table_name: String,
    pub where_clause: Option<Expression>,
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
