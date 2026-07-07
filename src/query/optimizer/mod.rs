use crate::{
    catalog::{index::IndexId, manager::Catalog},
    query::planner::{expression::PlannedExpression, plan::PlanNode},
};

pub struct Optimizer<'catalog, 'bpm> {
    catalog: &'catalog Catalog<'bpm>,
}

impl<'catalog, 'bpm> Optimizer<'catalog, 'bpm> {
    pub fn new(catalog: &'catalog Catalog<'bpm>) -> Self {
        Self { catalog }
    }

    pub fn optimize(&self, plan: PlanNode) -> PlanNode {
        let plan = self.optimize_merge_projection(plan);
        let plan = self.optimize_merge_filter_nlj(plan);
        let plan = self.optimize_nlj_as_hash_join(plan);
        let plan = self.optimize_nlj_as_index_join(plan);
        let plan = self.optimize_eliminate_true_filter(plan);
        let plan = self.optimize_merge_filter_scan(plan);
        let plan = self.optimize_order_by_as_index_scan(plan);
        let plan = self.optimize_seq_scan_as_index_scan(plan);
        let plan = self.optimize_column_pruning(plan);
        self.optimize_sort_limit_as_top_n(plan)
    }

    pub fn optimize_custom(&self, plan: PlanNode) -> PlanNode {
        plan
    }

    fn optimize_merge_projection(&self, plan: PlanNode) -> PlanNode {
        plan
    }

    fn optimize_merge_filter_nlj(&self, plan: PlanNode) -> PlanNode {
        plan
    }

    fn optimize_nlj_as_hash_join(&self, plan: PlanNode) -> PlanNode {
        plan
    }

    fn optimize_nlj_as_index_join(&self, plan: PlanNode) -> PlanNode {
        plan
    }

    fn optimize_eliminate_true_filter(&self, plan: PlanNode) -> PlanNode {
        plan
    }

    fn optimize_merge_filter_scan(&self, plan: PlanNode) -> PlanNode {
        plan
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

    fn optimize_order_by_as_index_scan(&self, plan: PlanNode) -> PlanNode {
        plan
    }

    fn optimize_seq_scan_as_index_scan(&self, plan: PlanNode) -> PlanNode {
        plan
    }

    fn match_index(&self, _table_name: &str, _index_key_idx: usize) -> Option<(IndexId, String)> {
        let _ = self.catalog;
        None
    }

    fn optimize_column_pruning(&self, plan: PlanNode) -> PlanNode {
        plan
    }

    fn optimize_sort_limit_as_top_n(&self, plan: PlanNode) -> PlanNode {
        plan
    }

    fn estimated_cardinality(&self, _table_name: &str) -> Option<usize> {
        None
    }
}
