use crate::{
    catalog::{column::Column, schema::Schema, table::TableId},
    query::{
        binder::{expression::ColumnRef, table_ref::BoundBaseTableRef},
        planner::expression::PlannedExpression,
    },
};

#[derive(Debug)]
pub struct PlanNode {
    pub output_schema: Schema,
    pub kind: PlanNodeKind,
}

impl PlanNode {
    pub fn output_schema(&self) -> &Schema {
        &self.output_schema
    }
}

#[derive(Debug)]
pub enum PlanNodeKind {
    SeqScan(SeqScanPlan),
    Filter(FilterPlan),
    Projection(ProjectionPlan),
    Values(ValuesPlan),
    Insert(InsertPlan),
    CreateTable(CreateTablePlan),
    Update(UpdatePlan),
    Delete(DeletePlan),
}

#[derive(Debug)]
pub struct SeqScanPlan {
    pub table_name: String,
    pub table_oid: TableId,
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

impl ProjectionPlan {
    // Infer the schema based on the exprs we are projecting
    pub fn infer_proj_schema(exprs: &[PlannedExpression]) -> Schema {
        let columns = exprs
            .iter()
            .map(|expr| expr.return_type.to_column("<unnamed>".into()))
            .collect::<Vec<_>>();
        Schema::new(&columns)
    }

    pub fn rename_schema(schema: &Schema, names: &[String]) -> Schema {
        assert_eq!(schema.num_columns(), names.len());

        let columns = schema
            .columns()
            .iter()
            .zip(names)
            .map(|(col, name)| col.with_new_name(name.clone()))
            .collect::<Vec<_>>();

        Schema::new(&columns)
    }
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
    pub primary_key_col_idxs: Vec<usize>,
}

#[derive(Debug)]
pub struct UpdatePlan {
    pub table_name: String,
    pub table_oid: TableId,
    pub table_schema: Schema,
    pub expressions: Vec<PlannedExpression>,
    pub child: Box<PlanNode>,
}

#[derive(Debug)]
pub struct DeletePlan {
    pub table_oid: TableId,
    pub child: Box<PlanNode>,
}
