use crate::{
    catalog::{schema::Schema, table::TableId},
    query::binder::expression::BoundExpression,
};

#[derive(Debug)]
pub enum TableRef {
    BaseTable(BoundBaseTableRef),
    ExprList(BoundExpressionListRef),
}

/**
 * A bound table ref type for single table. e.g.
* `SELECT x FROM y`, where `y` is `BoundBaseTableRef`.
 */
#[derive(Debug)]
pub struct BoundBaseTableRef {
    table_name: String,
    table_oid: TableId,
    alias: Option<String>,
    schema: Schema,
}

impl BoundBaseTableRef {
    pub fn new(
        table_name: String,
        table_oid: TableId,
        alias: Option<String>,
        schema: Schema,
    ) -> Self {
        Self {
            table_name,
            table_oid,
            alias,
            schema,
        }
    }

    pub fn tbl_name(&self) -> &str {
        &self.table_name
    }

    pub fn tbl_oid(&self) -> TableId {
        self.table_oid
    }

    pub fn bound_tbl_name(&self) -> &str {
        self.alias.as_ref().unwrap_or(&self.table_name)
    }

    pub fn schema(&self) -> &Schema {
        &self.schema
    }
}

#[derive(Debug)]
pub struct BoundExpressionListRef {
    // A unique identifier for this values list
    pub(crate) identifier: String,
    pub(crate) values: Vec<Vec<BoundExpression>>,
}

impl BoundExpressionListRef {
    pub fn new(identifier: String, values: Vec<Vec<BoundExpression>>) -> Self {
        Self { identifier, values }
    }
}

// TODO: our types of table refs later
