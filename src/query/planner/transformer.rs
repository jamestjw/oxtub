use crate::{
    catalog::manager::Catalog,
    query::{
        binder::{
            expression::BoundExpression,
            statement::{BoundSelect, BoundStatement},
            table_ref::TableRef,
        },
        planner::{
            error::PlannerError,
            expression::{
                ColumnValueExpression, ConstantValueExpression, PlannedExpression,
                PlannedExpressionKind,
            },
            plan::{FilterPlan, PlanNode, PlanNodeKind, ProjectionPlan, SeqScanPlan},
        },
    },
};

pub struct Planner<'catalog, 'bpm> {
    catalog: &'catalog Catalog<'bpm>,
}

impl<'catalog, 'bpm> Planner<'catalog, 'bpm> {
    pub fn new(catalog: &'catalog Catalog<'bpm>) -> Self {
        Self { catalog }
    }

    pub fn plan_statement(&self, stmt: BoundStatement) -> Result<PlanNode, PlannerError> {
        match stmt {
            BoundStatement::Select(bound_select) => self.plan_select(bound_select),
            BoundStatement::Insert(bound_insert) => todo!(),
            BoundStatement::Update(bound_update) => todo!(),
            BoundStatement::Delete(bound_delete) => todo!(),
            BoundStatement::Explain(bound_explain) => todo!(),
            BoundStatement::CreateTable(bound_create_table) => todo!(),
            BoundStatement::CreateIndex(bound_create_index) => todo!(),
            BoundStatement::DropTable(bound_drop_table) => todo!(),
            BoundStatement::DropIndex(bound_drop_index) => todo!(),
        }
    }

    fn plan_select(&self, stmt: BoundSelect) -> Result<PlanNode, PlannerError> {
        let plan = self.plan_table_ref(stmt.table)?;

        // Handle where statement (if any)
        let plan = match stmt.where_ {
            None => plan,
            Some(where_expr) => {
                let schema = plan.output_schema().clone();
                let (_, expr) = self.plan_expression(where_expr, vec![&plan])?;

                PlanNode {
                    output_schema: schema,
                    kind: PlanNodeKind::Filter(FilterPlan {
                        predicate: expr,
                        child: Box::new(plan),
                    }),
                }
            }
        };

        // Handle projections
        let plan = {
            let mut exprs = Vec::with_capacity(stmt.projection.len());
            let mut names = Vec::with_capacity(stmt.projection.len());

            for (idx, expr) in stmt.projection.into_iter().enumerate() {
                let (name, expr) = self.plan_expression(expr, vec![&plan])?;
                let name = name.unwrap_or_else(|| format!("__unnamed#{idx}"));

                exprs.push(expr);
                names.push(name);
            }

            let schema =
                ProjectionPlan::rename_schema(&ProjectionPlan::infer_proj_schema(&exprs), &names);

            PlanNode {
                output_schema: schema,
                kind: PlanNodeKind::Projection(ProjectionPlan {
                    expressions: exprs,
                    child: Box::new(plan),
                }),
            }
        };

        Ok(plan)
    }

    fn plan_table_ref(&self, tbl_ref: TableRef) -> Result<PlanNode, PlannerError> {
        match tbl_ref {
            TableRef::BaseTable(bound_base_table_ref) => {
                let tbl_info = self
                    .catalog
                    .get_tbl_by_name(bound_base_table_ref.tbl_name())?;

                // TODO: maybe handle internal tables?
                Ok(PlanNode {
                    output_schema: SeqScanPlan::infer_scan_schema(&bound_base_table_ref),
                    kind: PlanNodeKind::SeqScan(SeqScanPlan {
                        table_name: String::from(bound_base_table_ref.tbl_name()),
                        table_oid: tbl_info.table_oid(),
                    }),
                })
            }
            TableRef::ExprList(bound_expression_list_ref) => {
                panic!("planner does not support ExprList")
            }
        }
    }

    fn plan_expression(
        &self,
        expr: BoundExpression,
        children: Vec<&PlanNode>,
    ) -> Result<(Option<String>, PlannedExpression), PlannerError> {
        match expr {
            BoundExpression::Literal(value) => Ok((
                None,
                PlannedExpression {
                    return_type: value.get_type_as_col(),
                    kind: PlannedExpressionKind::ConstantValue(ConstantValueExpression { value }),
                },
            )),
            BoundExpression::Column(column_ref) => match children[..] {
                [child] => {
                    let col_name = column_ref.to_str();
                    let child_schema = child.output_schema();
                    let matched_columns = child_schema
                        .columns()
                        .iter()
                        .enumerate()
                        // Binder normalizes column refs to schema casing and scan schemas
                        // use the same qualified names.
                        .filter(|(_, col)| col.name() == col_name)
                        .collect::<Vec<_>>();

                    match matched_columns[..] {
                        [] => panic!("should not be possible as binder would have caught this?"),
                        [(idx, col)] => Ok((
                            Some(col_name),
                            PlannedExpression {
                                return_type: col.clone(),
                                kind: PlannedExpressionKind::ColumnValue(ColumnValueExpression {
                                    tuple_idx: 0,
                                    col_idx: idx,
                                }),
                            },
                        )),
                        _ => Err(PlannerError::AmbiguousColumn(col_name)),
                    }
                }
                [_left, _right] => todo!("binder doesnt support joins yet!"),
                _ => panic!("cannot occur"),
            },
            BoundExpression::BinaryOp { left, op, right } => todo!(),
            BoundExpression::UnaryOp { expr, op } => todo!(),
        }
    }
}
