use crate::{
    catalog::{index::IndexId, manager::Catalog},
    query::planner::{
        expression::{ColumnValueExpression, PlannedExpression, PlannedExpressionKind},
        plan::{PlanNode, PlanNodeKind, ProjectionPlan},
    },
};

pub struct Optimizer<'catalog, 'bpm> {
    catalog: &'catalog Catalog<'bpm>,
}

impl<'catalog, 'bpm> Optimizer<'catalog, 'bpm> {
    pub fn new(catalog: &'catalog Catalog<'bpm>) -> Self {
        Self { catalog }
    }

    pub fn optimize(&self, plan: &PlanNode) -> PlanNode {
        let plan = self.optimize_merge_projection(plan);
        let plan = self.optimize_merge_filter_nlj(&plan);
        let plan = self.optimize_nlj_as_hash_join(&plan);
        let plan = self.optimize_nlj_as_index_join(&plan);
        let plan = self.optimize_eliminate_true_filter(&plan);
        let plan = self.optimize_merge_filter_scan(&plan);
        let plan = self.optimize_order_by_as_index_scan(&plan);
        let plan = self.optimize_seq_scan_as_index_scan(&plan);
        let plan = self.optimize_column_pruning(&plan);
        self.optimize_sort_limit_as_top_n(&plan)
    }

    // When the projection selects the same stuff as what the child node would return
    fn optimize_merge_projection(&self, plan: &PlanNode) -> PlanNode {
        let optimized_children = plan
            .children()
            .into_iter()
            .map(|child| self.optimize_merge_projection(child))
            .collect::<Vec<_>>();
        let optimized_plan = plan.clone_with_children(optimized_children);

        match &optimized_plan.kind {
            PlanNodeKind::Projection(ProjectionPlan { expressions, child }) => {
                let child_schema = child.output_schema();
                if child_schema.num_columns() == expressions.len() {
                    for (child_column, projection_expr) in
                        child_schema.columns().iter().zip(expressions)
                    {
                        // TODO: varchar len difference should not necessarily be an issue
                        if child_column.sql_type() != projection_expr.return_type.sql_type {
                            return optimized_plan;
                        }
                    }

                    for idx in 0..expressions.len() {
                        match expressions.get(idx) {
                            Some(PlannedExpression {
                                kind:
                                    PlannedExpressionKind::ColumnValue(ColumnValueExpression {
                                        tuple_idx,
                                        col_idx,
                                    }),
                                ..
                            }) if *tuple_idx == 0 && *col_idx == idx => (),
                            _ => return optimized_plan,
                        }
                    }

                    let mut plan = child.as_ref().clone();
                    plan.output_schema = optimized_plan.output_schema().clone();
                    plan
                } else {
                    optimized_plan
                }
            }
            _ => optimized_plan,
        }
    }

    fn optimize_merge_filter_nlj(&self, plan: &PlanNode) -> PlanNode {
        plan.clone()
    }

    fn optimize_nlj_as_hash_join(&self, plan: &PlanNode) -> PlanNode {
        plan.clone()
    }

    fn optimize_nlj_as_index_join(&self, plan: &PlanNode) -> PlanNode {
        plan.clone()
    }

    fn optimize_eliminate_true_filter(&self, plan: &PlanNode) -> PlanNode {
        plan.clone()
    }

    fn optimize_merge_filter_scan(&self, plan: &PlanNode) -> PlanNode {
        plan.clone()
    }

    fn rewrite_expression_for_join(
        &self,
        expr: PlannedExpression,
        _left_column_count: usize,
        _right_column_count: usize,
    ) -> PlannedExpression {
        expr
    }

    fn is_predicate_true(&self, _expr: &PlannedExpression) -> bool {
        false
    }

    fn optimize_order_by_as_index_scan(&self, plan: &PlanNode) -> PlanNode {
        plan.clone()
    }

    fn optimize_seq_scan_as_index_scan(&self, plan: &PlanNode) -> PlanNode {
        plan.clone()
    }

    fn match_index(&self, _table_name: &str, _index_key_idx: usize) -> Option<(IndexId, String)> {
        let _ = self.catalog;
        None
    }

    fn optimize_column_pruning(&self, plan: &PlanNode) -> PlanNode {
        plan.clone()
    }

    fn optimize_sort_limit_as_top_n(&self, plan: &PlanNode) -> PlanNode {
        plan.clone()
    }

    fn estimated_cardinality(&self, _table_name: &str) -> Option<usize> {
        None
    }
}

#[cfg(test)]
mod tests {
    use tempfile::NamedTempFile;

    use crate::{
        buffer::bpm::BufferPoolManager,
        catalog::{column::Column, manager::Catalog, schema::Schema, types::SqlType},
        query::{
            binder::transformer::Binder,
            optimizer::Optimizer,
            parser::parse_sql,
            planner::{plan::PlanNodeKind, transformer::Planner},
        },
        storage::disk::disk_manager::DiskManager,
    };

    fn setup_bpm(pool_size: usize) -> BufferPoolManager {
        let file = NamedTempFile::new().unwrap();
        let disk_manager = DiskManager::new(file.path().to_path_buf()).unwrap();
        BufferPoolManager::new(pool_size, disk_manager)
    }

    fn create_users_table(catalog: &mut Catalog<'_>) {
        let schema = Schema::new(&[
            Column::new_static("id".to_string(), SqlType::Integer),
            Column::new_variable("name".to_string(), SqlType::Varchar, 32),
        ]);

        catalog.create_tbl("users".to_string(), schema).unwrap();
    }

    fn plan_sql(catalog: &Catalog<'_>, sql: &str) -> crate::query::planner::plan::PlanNode {
        let statement = parse_sql(sql).unwrap();
        let binder = Binder::new(catalog);
        let bound = binder.bind_statement(statement).unwrap();
        let planner = Planner::new(catalog);

        planner.plan_statement(bound).unwrap()
    }

    #[test]
    fn removes_superfluous_projection() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);

        let plan = plan_sql(&catalog, "select id, name from users");

        assert!(matches!(plan.kind, PlanNodeKind::Projection(_)));

        let optimized = Optimizer::new(&catalog).optimize(&plan);

        assert!(matches!(optimized.kind, PlanNodeKind::SeqScan(_)));
        assert_eq!(optimized.output_schema(), plan.output_schema());
    }
}
