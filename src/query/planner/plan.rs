use crate::{
    catalog::{column::Column, schema::Schema, table::TableId},
    query::{
        binder::{expression::ColumnRef, table_ref::BoundBaseTableRef},
        planner::expression::PlannedExpression,
    },
};

#[derive(Debug)]
pub enum PlanNode {
    SeqScan(SeqScanPlan),
    Filter(FilterPlan),
    Projection(ProjectionPlan),
    Values(ValuesPlan),
    Insert(InsertPlan),
    CreateTable(CreateTablePlan),
}

#[derive(Debug)]
pub struct SeqScanPlan {
    pub table_name: String,
    pub table_oid: TableId,
    pub output_schema: Schema,
}

impl SeqScanPlan {
    // Infer the schema of doing a sequential scan on a table
    pub fn infer_scan_schema(base_table: &BoundBaseTableRef) -> Schema {
        let columns = base_table
            .schema()
            .columns()
            .iter()
            .map(|col| {
                col.with_new_name(format!(
                    "{bound_tbl_name}.{col_name}",
                    bound_tbl_name = base_table.bound_tbl_name(),
                    col_name = col.name()
                ))
            })
            .collect::<Vec<Column>>();
        Schema::new(&columns)
    }
}

#[derive(Debug)]
pub struct FilterPlan {
    pub predicate: PlannedExpression,
    pub child: Box<PlanNode>,
}

#[derive(Debug)]
pub struct ProjectionPlan {
    pub expressions: Vec<PlannedExpression>,
    pub child: Box<PlanNode>,
}

#[derive(Debug)]
pub struct ValuesPlan {
    pub rows: Vec<Vec<PlannedExpression>>,
}

#[derive(Debug)]
pub struct InsertPlan {
    pub table_name: String,
    pub table_oid: TableId,
    pub table_schema: Schema,
    pub columns: Vec<ColumnRef>,
    pub child: Box<PlanNode>,
}

#[derive(Debug)]
pub struct CreateTablePlan {
    pub name: String,
    pub columns: Vec<Column>,
    pub primary_key_cols: Vec<String>,
}
