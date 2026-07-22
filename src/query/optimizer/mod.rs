use crate::{
    catalog::{index::IndexId, manager::Catalog, schema::Schema},
    query::{
        planner::{
            expression::{
                ColumnValueExpression, ComparisonExpression, ComparisonType, ExpressionType,
                LogicExpression, LogicType, PlannedExpression, PlannedExpressionKind,
            },
            plan::{
                FilterPlan, NestedIndexJoinPlan, NestedLoopJoinPlan, PlanNode, PlanNodeKind,
                ProjectionPlan, SeqScanPlan,
            },
        },
        table_ref::JoinType,
    },
};

pub struct Optimizer<'catalog, 'bpm> {
    catalog: &'catalog Catalog<'bpm>,
}

impl<'catalog, 'bpm> Optimizer<'catalog, 'bpm> {
    pub fn new(catalog: &'catalog Catalog<'bpm>) -> Self {
        Self { catalog }
    }

    fn build_index_expressions(schema: &Schema, col_idxs: &[usize]) -> Vec<PlannedExpression> {
        col_idxs
            .iter()
            .map(|col_idx| PlannedExpression {
                return_type: ExpressionType::from_column(&schema.columns()[*col_idx]),
                kind: PlannedExpressionKind::ColumnValue(ColumnValueExpression {
                    tuple_idx: 0,
                    col_idx: *col_idx,
                }),
            })
            .collect()
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
        let optimized_children = plan
            .children()
            .into_iter()
            .map(|child| self.optimize_nlj_as_index_join(child))
            .collect::<Vec<_>>();
        let optimized_plan = plan.clone_with_children(optimized_children);

        match &optimized_plan.kind {
            PlanNodeKind::NestedLoopJoin(NestedLoopJoinPlan {
                left: left_plan,
                right: right_plan,
                join_type: join_type @ (JoinType::Left | JoinType::Inner),
                predicate: Some(predicate),
            }) => {
                // only works if both children are SeqScans
                match (&left_plan.kind, &right_plan.kind) {
                    (
                        PlanNodeKind::SeqScan(SeqScanPlan {
                            filter_predicate: left_filter_predicate,
                            table_name: left_table_name,
                            table_oid: left_table_oid,
                        }),
                        PlanNodeKind::SeqScan(SeqScanPlan {
                            filter_predicate: right_filter_predicate,
                            table_name: right_table_name,
                            table_oid: right_table_oid,
                        }),
                    ) => {
                        let Some((left_col_idxs, right_col_idxs)) =
                            Self::extract_index_exprs_for_index_join_predicate(predicate)
                        else {
                            return optimized_plan;
                        };

                        // TODO: if we only have indexes on a subset of these columns, it should
                        // still be possible to do an index join!

                        // TODO: also support using an index on the left table without changing
                        // the join output schema/order.
                        let Some((right_index_oid, right_index_name)) =
                            self.matches_index_of_tbl(right_table_name, &right_col_idxs)
                        else {
                            return optimized_plan;
                        };

                        let index_expressions = Self::build_index_expressions(
                            left_plan.output_schema(),
                            &left_col_idxs,
                        );

                        PlanNode {
                            output_schema: optimized_plan.output_schema().clone(),
                            kind: PlanNodeKind::NestedIndexJoin(NestedIndexJoinPlan {
                                child: Box::new(left_plan.as_ref().clone()),
                                index_expressions,
                                inner_table_oid: *right_table_oid,
                                inner_table_index_oid: right_index_oid,
                                inner_table_index_name: right_index_name,
                                inner_table_schema: right_plan.output_schema().clone(),
                                join_type: *join_type,
                            }),
                        }
                    }
                    _ => optimized_plan,
                }
            }
            _ => optimized_plan,
        }
    }

    fn extract_index_exprs_for_index_join_predicate(
        predicate: &PlannedExpression,
    ) -> Option<(Vec<usize>, Vec<usize>)> {
        // TODO: we can actually support a wider range of expressions e.g. we could gather the CVE
        // from the tables, and then evaluate the remaining predicates separately in the executor.
        match &predicate.kind {
            PlannedExpressionKind::Comparison(ComparisonExpression {
                left,
                comparison_type: ComparisonType::Eq,
                right,
            }) => match (&left.kind, &right.kind) {
                (
                    PlannedExpressionKind::ColumnValue(ColumnValueExpression {
                        tuple_idx: left_tuple_idx,
                        col_idx: left_col_idx,
                    }),
                    PlannedExpressionKind::ColumnValue(ColumnValueExpression {
                        tuple_idx: right_tuple_idx,
                        col_idx: right_col_idx,
                    }),
                ) => {
                    // Ensure that the CVE from the left plan is on the left, tolerate inversions
                    let (left_col_idx, right_col_idx) = match (*left_tuple_idx, *right_tuple_idx) {
                        (0, 1) => (left_col_idx, right_col_idx),
                        (1, 0) => (right_col_idx, left_col_idx),
                        _ => return None,
                    };
                    Some((vec![*left_col_idx], vec![*right_col_idx]))
                }
                _ => None,
            },
            PlannedExpressionKind::Logic(LogicExpression {
                left,
                logic_type: LogicType::And,
                right,
            }) => {
                match (
                    Self::extract_index_exprs_for_index_join_predicate(left),
                    Self::extract_index_exprs_for_index_join_predicate(right),
                ) {
                    (Some((left1, right1)), Some((left2, right2))) => Some((
                        [left1.as_slice(), left2.as_slice()].concat(),
                        [right1.as_slice(), right2.as_slice()].concat(),
                    )),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn matches_index_of_tbl(
        &self,
        table_name: &str,
        col_idxes: &[usize],
    ) -> Option<(IndexId, String)> {
        match self.catalog.get_table_indexes(table_name) {
            // This should never error in reality
            Err(_) => None,
            Ok(index_infos) => index_infos
                .iter()
                .find(|info| info.index.metadata().key_attrs == col_idxes)
                .map(|info| (info.oid(), info.index.metadata().name.clone())),
        }
    }

    fn optimize_eliminate_true_filter(&self, plan: &PlanNode) -> PlanNode {
        // TODO: this eliminates `WHERE true` filters, this probably isn't
        // very useful until we have constant folding, i.e. optimising
        // `1 = 1` to `True` and simplifying filters with logical operators
        // when some expressions are trivially true or false.
        plan.clone()
    }

    fn optimize_merge_filter_scan(&self, plan: &PlanNode) -> PlanNode {
        // merge filter into filter_predicate of seq scan plan node
        let optimized_children = plan
            .children()
            .into_iter()
            .map(|child| self.optimize_merge_filter_scan(child))
            .collect::<Vec<_>>();
        let optimized_plan = plan.clone_with_children(optimized_children);

        match &optimized_plan.kind {
            PlanNodeKind::Filter(FilterPlan { predicate, child }) => match &child.kind {
                PlanNodeKind::SeqScan(SeqScanPlan {
                    table_name,
                    table_oid,
                    filter_predicate,
                }) if filter_predicate.is_none()
                // the SeqScanPlan produced by the planner should not have a filter predicate so it
                // should always be true (for now), if it's not the case, we can always use a
                // conjunction to combine the predicates, though for now this shouldn't open so we
                // skip it
                => PlanNode {
                    output_schema: optimized_plan.output_schema().clone(),
                    kind: PlanNodeKind::SeqScan(SeqScanPlan {
                        table_name: table_name.clone(),
                        table_oid: *table_oid,
                        filter_predicate: Some(predicate.clone()),
                    }),
                },
                _ => optimized_plan,
            },
            _ => optimized_plan,
        }
    }

    fn rewrite_expression_for_join(
        &self,
        expr: PlannedExpression,
        _left_column_count: usize,
        _right_column_count: usize,
    ) -> PlannedExpression {
        expr
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
    use expect_test::expect;

    use crate::{
        catalog::{column::Column, manager::Catalog, schema::Schema, types::SqlType},
        query::{
            binder::transformer::Binder,
            optimizer::Optimizer,
            parser::parse_sql,
            planner::{plan::PlanNodeKind, transformer::Planner},
        },
        testing::setup_bpm,
    };

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

    #[test]
    fn pushes_filter_predicate_into_seq_scan() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);

        let plan = plan_sql(&catalog, "select id, name from users where id = 1");
        let optimized = Optimizer::new(&catalog).optimize(&plan);

        expect![[r#"
            PlanNode {
                output_schema: Schema {
                    inlined_storage_size: 9,
                    columns: [
                        Column {
                            name: "users.id",
                            sql_type: Integer,
                            value_offset: 1,
                            size: Inline(
                                4,
                            ),
                        },
                        Column {
                            name: "users.name",
                            sql_type: Varchar,
                            value_offset: 5,
                            size: Variable(
                                32,
                            ),
                        },
                    ],
                    uninlined_columns: [
                        1,
                    ],
                },
                kind: SeqScan(
                    SeqScanPlan {
                        table_name: "users",
                        table_oid: 0,
                        filter_predicate: Some(
                            PlannedExpression {
                                return_type: ExpressionType {
                                    sql_type: Boolean,
                                    varchar_size: None,
                                },
                                kind: Comparison(
                                    ComparisonExpression {
                                        left: PlannedExpression {
                                            return_type: ExpressionType {
                                                sql_type: Integer,
                                                varchar_size: None,
                                            },
                                            kind: ColumnValue(
                                                ColumnValueExpression {
                                                    tuple_idx: 0,
                                                    col_idx: 0,
                                                },
                                            ),
                                        },
                                        comparison_type: Eq,
                                        right: PlannedExpression {
                                            return_type: ExpressionType {
                                                sql_type: Integer,
                                                varchar_size: None,
                                            },
                                            kind: ConstantValue(
                                                ConstantValueExpression {
                                                    value: Integer(
                                                        1,
                                                    ),
                                                },
                                            ),
                                        },
                                    },
                                ),
                            },
                        ),
                    },
                ),
            }"#]]
        .assert_eq(&format!("{optimized:#?}"));
    }
}
