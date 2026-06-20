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

pub enum StatementType {
    Select,
    Insert,
    Update,
    Delete,
    Explain,
    CreateTable,
    CreateIndex,
    DropTable,
    DropIndex,
}

pub struct BoundSelect;

pub struct BoundInsert;

pub struct BoundUpdate;

pub struct BoundDelete;

pub struct BoundExplain;

pub struct BoundCreateTable;

pub struct BoundCreateIndex;

pub struct BoundDropTable;

pub struct BoundDropIndex;
