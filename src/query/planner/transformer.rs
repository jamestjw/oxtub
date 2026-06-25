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
            expression::PlannedExpression,
            plan::{FilterPlan, PlanNode, PlanNodeKind, SeqScanPlan},
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
            BoundStatement::Select(bound_select) => todo!(),
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
        todo!()
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
        todo!()
    }
}
