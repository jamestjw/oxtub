use crate::{
    catalog::{column::Column, index::IndexId, schema::Schema, table::TableId},
    query::{
        binder::table_ref::BoundBaseTableRef, planner::expression::PlannedExpression,
        table_ref::JoinType,
    },
};

#[derive(Debug, Clone)]
pub struct PlanNode {
    pub output_schema: Schema,
    pub kind: PlanNodeKind,
}

impl PlanNode {
    pub fn output_schema(&self) -> &Schema {
        &self.output_schema
    }

    pub fn clone_with_children(&self, mut children: Vec<PlanNode>) -> Self {
        let kind = match &self.kind {
            PlanNodeKind::SeqScan(seq_scan_plan) if children.is_empty() => {
                PlanNodeKind::SeqScan(seq_scan_plan.clone())
            }
            PlanNodeKind::Filter(FilterPlan {
                predicate,
                child: _,
            }) if children.len() == 1 => PlanNodeKind::Filter(FilterPlan {
                predicate: predicate.clone(),
                child: Box::new(children.pop().unwrap()),
            }),
            PlanNodeKind::Projection(ProjectionPlan {
                expressions,
                child: _,
            }) if children.len() == 1 => PlanNodeKind::Projection(ProjectionPlan {
                expressions: expressions.clone(),
                child: Box::new(children.pop().unwrap()),
            }),
            PlanNodeKind::Values(values_plan) if children.is_empty() => {
                PlanNodeKind::Values(values_plan.clone())
            }
            PlanNodeKind::Insert(insert_plan) if children.len() == 1 => {
                PlanNodeKind::Insert(InsertPlan {
                    table_name: insert_plan.table_name.clone(),
                    table_oid: insert_plan.table_oid,
                    table_schema: insert_plan.table_schema.clone(),
                    target_col_idxs: insert_plan.target_col_idxs.clone(),
                    child: Box::new(children.pop().unwrap()),
                })
            }
            PlanNodeKind::CreateTable(create_table_plan) if children.is_empty() => {
                PlanNodeKind::CreateTable(create_table_plan.clone())
            }
            PlanNodeKind::Update(update_plan) if children.len() == 1 => {
                PlanNodeKind::Update(UpdatePlan {
                    table_name: update_plan.table_name.clone(),
                    table_oid: update_plan.table_oid,
                    table_schema: update_plan.table_schema.clone(),
                    expressions: update_plan.expressions.clone(),
                    child: Box::new(children.pop().unwrap()),
                })
            }
            PlanNodeKind::Delete(delete_plan) if children.len() == 1 => {
                PlanNodeKind::Delete(DeletePlan {
                    table_oid: delete_plan.table_oid,
                    child: Box::new(children.pop().unwrap()),
                })
            }
            PlanNodeKind::NestedLoopJoin(nested_loop_join_plan) if children.len() == 2 => {
                let right = children.pop().unwrap();
                let left = children.pop().unwrap();
                PlanNodeKind::NestedLoopJoin(NestedLoopJoinPlan {
                    left: Box::new(left),
                    right: Box::new(right),
                    join_type: nested_loop_join_plan.join_type,
                    predicate: nested_loop_join_plan.predicate.clone(),
                })
            }
            PlanNodeKind::NestedIndexJoin(nested_index_join_plan) if children.len() == 1 => {
                PlanNodeKind::NestedIndexJoin(NestedIndexJoinPlan {
                    child: Box::new(children.pop().unwrap()),
                    predicate_expressions: nested_index_join_plan.predicate_expressions.clone(),
                    inner_table_oid: nested_index_join_plan.inner_table_oid,
                    inner_table_index_oid: nested_index_join_plan.inner_table_index_oid,
                    inner_table_index_name: nested_index_join_plan.inner_table_index_name.clone(),
                    inner_table_schema: nested_index_join_plan.inner_table_schema.clone(),
                    join_type: nested_index_join_plan.join_type,
                })
            }
            _ => panic!("unexpected shape"),
        };

        Self {
            output_schema: self.output_schema.clone(),
            kind,
        }
    }

    pub fn children(&self) -> Vec<&PlanNode> {
        match &self.kind {
            PlanNodeKind::SeqScan(_) => vec![],
            PlanNodeKind::Filter(filter) => vec![filter.child.as_ref()],
            PlanNodeKind::Projection(projection) => vec![projection.child.as_ref()],
            PlanNodeKind::Values(_) => vec![],
            PlanNodeKind::Insert(insert) => vec![insert.child.as_ref()],
            PlanNodeKind::CreateTable(_) => vec![],
            PlanNodeKind::Update(update) => vec![update.child.as_ref()],
            PlanNodeKind::Delete(delete) => vec![delete.child.as_ref()],
            PlanNodeKind::NestedLoopJoin(nested_loop_join_plan) => {
                vec![
                    nested_loop_join_plan.left.as_ref(),
                    nested_loop_join_plan.right.as_ref(),
                ]
            }
            PlanNodeKind::NestedIndexJoin(nested_index_join_plan) => {
                vec![nested_index_join_plan.child.as_ref()]
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum PlanNodeKind {
    SeqScan(SeqScanPlan),
    Filter(FilterPlan),
    Projection(ProjectionPlan),
    Values(ValuesPlan),
    Insert(InsertPlan),
    CreateTable(CreateTablePlan),
    Update(UpdatePlan),
    Delete(DeletePlan),
    NestedLoopJoin(NestedLoopJoinPlan),
    NestedIndexJoin(NestedIndexJoinPlan),
}

#[derive(Debug, Clone)]
pub struct SeqScanPlan {
    pub table_name: String,
    pub table_oid: TableId,
    pub filter_predicate: Option<PlannedExpression>,
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

#[derive(Debug, Clone)]
pub struct FilterPlan {
    pub predicate: PlannedExpression,
    pub child: Box<PlanNode>,
}

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct ValuesPlan {
    pub rows: Vec<Vec<PlannedExpression>>,
}

#[derive(Debug, Clone)]
pub struct InsertPlan {
    pub table_name: String,
    pub table_oid: TableId,
    pub table_schema: Schema,
    pub target_col_idxs: Vec<usize>,
    pub child: Box<PlanNode>,
}

#[derive(Debug, Clone)]
pub struct CreateTablePlan {
    pub name: String,
    pub columns: Vec<Column>,
    pub primary_key_col_idxs: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct UpdatePlan {
    pub table_name: String,
    pub table_oid: TableId,
    pub table_schema: Schema,
    pub expressions: Vec<PlannedExpression>,
    pub child: Box<PlanNode>,
}

#[derive(Debug, Clone)]
pub struct DeletePlan {
    pub table_oid: TableId,
    pub child: Box<PlanNode>,
}

#[derive(Debug, Clone)]
pub struct NestedLoopJoinPlan {
    pub left: Box<PlanNode>,
    pub right: Box<PlanNode>,
    pub join_type: JoinType,
    pub predicate: Option<PlannedExpression>,
}

impl NestedLoopJoinPlan {
    pub fn infer_join_schema(left: &PlanNode, right: &PlanNode) -> Schema {
        let mut columns = Vec::from(left.output_schema().columns());
        columns.extend_from_slice(right.output_schema().columns());
        Schema::new(&columns)
    }
}

#[derive(Debug, Clone)]
pub struct NestedIndexJoinPlan {
    // outer table of the index join
    pub child: Box<PlanNode>,
    // Expressions of the outer table that need to match the index of the inner table.
    // Vec as we could be using a composite index
    pub predicate_expressions: Vec<PlannedExpression>,
    pub inner_table_oid: TableId,
    pub inner_table_index_oid: IndexId,
    pub inner_table_index_name: String,
    // TODO: verify if schema is needed
    pub inner_table_schema: Schema,
    pub join_type: JoinType,
    // TODO: we should also support joins that use more than indexed columns
}
