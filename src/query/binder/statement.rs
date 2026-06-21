use crate::catalog::column::Column;

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
pub struct BoundSelect;

#[derive(Debug)]
pub struct BoundInsert;

#[derive(Debug)]
pub struct BoundUpdate;

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
