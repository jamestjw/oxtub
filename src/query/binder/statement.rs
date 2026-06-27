use crate::{
    catalog::column::Column,
    query::binder::{
        expression::{BoundExpression, ColumnRef},
        table_ref::{BoundBaseTableRef, BoundExpressionListRef, TableRef},
    },
};

#[derive(Debug)]
pub enum BoundStatement {
    Select(BoundSelect),
    Insert(BoundInsert),
    Update(BoundUpdate),
    Delete(BoundDelete),
    Explain(BoundExplain),
    CreateTable(BoundCreateTable),
    CreateIndex(BoundCreateIndex),
    DropTable(BoundDropTable),
    DropIndex(BoundDropIndex),
}

#[derive(Debug)]
pub struct BoundSelect {
    pub table: TableRef,
    pub projection: Vec<BoundExpression>,
    pub where_: Option<BoundExpression>,
}

#[derive(Debug)]
pub struct BoundInsert {
    pub table: BoundBaseTableRef,
    pub columns: Vec<ColumnRef>,
    pub bound_exprs: BoundExpressionListRef,
}

#[derive(Debug)]
pub struct BoundUpdate {
    pub table: BoundBaseTableRef,
    pub filter_expr: BoundExpression,
    pub target_expr: Vec<(ColumnRef, BoundExpression)>,
}

#[derive(Debug)]
pub struct BoundDelete;

#[derive(Debug)]
pub struct BoundExplain;

#[derive(Debug)]
pub struct BoundCreateTable {
    pub name: String,
    pub columns: Vec<Column>,
    pub primary_key_cols: Vec<String>,
}

#[derive(Debug)]
pub struct BoundCreateIndex;

#[derive(Debug)]
pub struct BoundDropTable;

#[derive(Debug)]
pub struct BoundDropIndex;
