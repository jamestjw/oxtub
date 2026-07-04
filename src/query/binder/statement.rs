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
    pub source: BoundInsertSource,
}

#[derive(Debug)]
pub enum BoundInsertSource {
    Values(BoundExpressionListRef),
    Select(BoundSelect),
}

#[derive(Debug)]
pub struct BoundUpdate {
    pub table: BoundBaseTableRef,
    pub filter_expr: Option<BoundExpression>,
    pub target_exprs: Vec<(ColumnRef, BoundExpression)>,
}

#[derive(Debug)]
pub struct BoundDelete {
    pub table: BoundBaseTableRef,
    pub filter_expr: Option<BoundExpression>,
}

#[derive(Debug)]
pub struct BoundExplain;

#[derive(Debug)]
pub struct BoundCreateTable {
    pub name: String,
    pub columns: Vec<Column>,
    pub primary_key_col_idxs: Vec<usize>,
}

#[derive(Debug)]
pub struct BoundCreateIndex;

#[derive(Debug)]
pub struct BoundDropTable;

#[derive(Debug)]
pub struct BoundDropIndex;
