use crate::{
    catalog::{column::Column, manager::Catalog, schema::Schema, types::SqlType},
    query::{
        binder::{
            expression::{BoundExpression, ColumnRef},
            statement::{
                BoundDelete, BoundInsert, BoundInsertSource, BoundSelect, BoundStatement,
                BoundUpdate,
            },
            table_ref::{BoundExpressionListRef, BoundTableRef},
        },
        expression::{BinaryOperator, UnaryOperator},
        planner::{
            error::PlannerError,
            expression::{
                ArithmeticExpression, ArithmeticType, ColumnValueExpression, ComparisonExpression,
                ComparisonType, ConstantValueExpression, ExpressionType, LogicExpression,
                LogicType, NegateExpression, NotExpression, NullCheckExpression, NullCheckType,
                PlannedExpression, PlannedExpressionKind,
            },
            plan::{
                DeletePlan, FilterPlan, InsertPlan, PlanNode, PlanNodeKind, ProjectionPlan,
                SeqScanPlan, UpdatePlan, ValuesPlan,
            },
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
            BoundStatement::Insert(bound_insert) => self.plan_insert(bound_insert),
            BoundStatement::Update(bound_update) => self.plan_update(bound_update),
            BoundStatement::Delete(bound_delete) => self.plan_delete(bound_delete),
            BoundStatement::Explain(bound_explain) => todo!(),
            BoundStatement::CreateTable(bound_create_table) => todo!(),
            BoundStatement::CreateIndex(bound_create_index) => todo!(),
            BoundStatement::DropTable(bound_drop_table) => todo!(),
            BoundStatement::DropIndex(bound_drop_index) => todo!(),
        }
    }

    fn plan_update(&self, bound_update: BoundUpdate) -> Result<PlanNode, PlannerError> {
        let table_name = bound_update.table.tbl_name().to_owned();
        let table_oid = bound_update.table.tbl_oid();
        let table_schema = bound_update.table.schema().clone();

        let planned_table = self.plan_table_ref(BoundTableRef::BaseTable(bound_update.table))?;
        let condition = match bound_update.filter_expr {
            Some(expr) => {
                let (_, expr) = self.plan_expression(expr, &[&planned_table])?;
                Some(expr)
            }
            None => None,
        };

        let filtered_node = match condition {
            Some(expr) => PlanNode {
                output_schema: planned_table.output_schema().clone(),
                kind: PlanNodeKind::Filter(FilterPlan {
                    predicate: expr,
                    child: Box::new(planned_table),
                }),
            },
            None => planned_table,
        };

        // We build target_exprs by just referencing the original columns,
        // then we put in the new values for columns that have been modified
        let mut target_exprs: Vec<PlannedExpression> = filtered_node
            .output_schema()
            .columns()
            .iter()
            .enumerate()
            .map(|(idx, col)| PlannedExpression {
                return_type: ExpressionType::from_column(col),
                kind: PlannedExpressionKind::ColumnValue(ColumnValueExpression {
                    tuple_idx: 0,
                    col_idx: idx,
                }),
            })
            .collect();

        let scope = &[&filtered_node];
        for (col, expr) in bound_update.target_exprs {
            let (_, target_expr) = self.plan_expression(expr, scope)?;
            let (_, col_expr, expr_type) = self.plan_column_ref(col, scope)?;

            // TODO: we don't need pure equality, we could always save an INTEGER into
            // a column with type BIGINT for instance. In reality, we don't want equality
            // but some notion of compatibility.
            if expr_type.sql_type != target_expr.return_type.sql_type {
                return Err(PlannerError::UpdateSchemaMismatch);
            }

            target_exprs[col_expr.col_idx] = target_expr;
        }

        // Number of rows updated
        let output_schema = Schema::new(&[Column::new_static(
            "__oxtub_internal.update_rows".to_string(),
            SqlType::Integer,
        )]);

        Ok(PlanNode {
            kind: PlanNodeKind::Update(UpdatePlan {
                table_name,
                table_oid,
                table_schema,
                expressions: target_exprs,
                child: Box::new(filtered_node),
            }),
            output_schema,
        })
    }

    fn plan_delete(&self, bound_delete: BoundDelete) -> Result<PlanNode, PlannerError> {
        let table_oid = bound_delete.table.tbl_oid();

        let planned_table = self.plan_table_ref(BoundTableRef::BaseTable(bound_delete.table))?;
        let condition = match bound_delete.filter_expr {
            Some(expr) => {
                let (_, expr) = self.plan_expression(expr, &[&planned_table])?;
                Some(expr)
            }
            None => None,
        };

        let filtered_node = match condition {
            Some(expr) => PlanNode {
                output_schema: planned_table.output_schema().clone(),
                kind: PlanNodeKind::Filter(FilterPlan {
                    predicate: expr,
                    child: Box::new(planned_table),
                }),
            },
            None => planned_table,
        };

        // Number of rows deleted
        let output_schema = Schema::new(&[Column::new_static(
            "__oxtub_internal.delete_rows".to_string(),
            SqlType::Integer,
        )]);

        Ok(PlanNode {
            kind: PlanNodeKind::Delete(DeletePlan {
                table_oid,
                child: Box::new(filtered_node),
            }),
            output_schema,
        })
    }

    fn plan_insert(&self, stmt: BoundInsert) -> Result<PlanNode, PlannerError> {
        let child = match stmt.source {
            BoundInsertSource::Values(values) => self.plan_bound_expression_list(values)?,
            BoundInsertSource::Select(select) => self.plan_select(select)?,
        };
        let insert_columns = stmt.columns;
        let table_schema = stmt.table.schema();
        let child_schema = child.output_schema();

        if insert_columns.len() != child_schema.columns().len() {
            return Err(PlannerError::InsertSchemaMismatch);
        }

        let mut target_col_idxs = Vec::with_capacity(insert_columns.len());

        for (insert_col, child_col) in insert_columns.iter().zip(child_schema.columns()) {
            let insert_col_name = match insert_col {
                ColumnRef::Unqualified { column } => column,
                ColumnRef::TableQualified { column, .. }
                | ColumnRef::SchemaTableQualified { column, .. } => column,
            };

            let target_col_idx = table_schema
                .columns()
                .iter()
                .position(|col| col.name() == insert_col_name)
                .expect("binder should have resolved insert columns");
            let target_col = &table_schema.columns()[target_col_idx];

            if target_col.sql_type() != child_col.sql_type() {
                return Err(PlannerError::InsertSchemaMismatch);
            }
            if target_col.sql_type().is_varlen()
                && child_col.declared_size() > target_col.declared_size()
            {
                return Err(PlannerError::InsertSchemaMismatch);
            }

            target_col_idxs.push(target_col_idx);
        }

        // Output of the insert statement is the number of rows inserted
        let output_col = Column::new_static(
            String::from("__oxtub_internal.insert_rows"),
            SqlType::Integer,
        );

        Ok(PlanNode {
            output_schema: Schema::new(&[output_col]),
            kind: PlanNodeKind::Insert(InsertPlan {
                table_name: stmt.table.tbl_name().to_string(),
                table_oid: stmt.table.tbl_oid(),
                table_schema: stmt.table.schema().clone(),
                target_col_idxs,
                child: Box::new(child),
            }),
        })
    }

    fn plan_bound_expression_list(
        &self,
        expr_list: BoundExpressionListRef,
    ) -> Result<PlanNode, PlannerError> {
        assert!(expr_list.values.len() > 0);

        let planned_rows: Vec<Vec<PlannedExpression>> = expr_list
            .values
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|expr| {
                        let (_, expr) = self.plan_expression(expr, &[])?;
                        Ok(expr)
                    })
                    .collect::<Result<Vec<_>, PlannerError>>()
            })
            .collect::<Result<Vec<_>, _>>()?;

        // TODO: validate every row has the same types before inferring the schema from the first row.
        // Make schema from first row since all rows should have the same schema.
        let columns = planned_rows[0]
            .iter()
            .enumerate()
            .map(|(i, col)| {
                col.return_type
                    .to_column(format!("{id}.{i}", id = expr_list.identifier))
            })
            .collect::<Vec<_>>();

        Ok(PlanNode {
            output_schema: Schema::new(&columns),
            kind: PlanNodeKind::Values(ValuesPlan { rows: planned_rows }),
        })
    }

    fn plan_select(&self, stmt: BoundSelect) -> Result<PlanNode, PlannerError> {
        let plan = self.plan_table_ref(stmt.table)?;

        // Handle where statement (if any)
        let plan = match stmt.where_ {
            None => plan,
            Some(where_expr) => {
                let schema = plan.output_schema().clone();
                let (_, expr) = self.plan_expression(where_expr, &[&plan])?;

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
                let (name, expr) = self.plan_expression(expr, &[&plan])?;
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

    fn plan_table_ref(&self, tbl_ref: BoundTableRef) -> Result<PlanNode, PlannerError> {
        match tbl_ref {
            BoundTableRef::BaseTable(bound_base_table_ref) => {
                let tbl_info = self
                    .catalog
                    .get_tbl_by_name(bound_base_table_ref.tbl_name())?;

                // TODO: maybe handle internal tables?
                Ok(PlanNode {
                    output_schema: SeqScanPlan::infer_scan_schema(&bound_base_table_ref),
                    kind: PlanNodeKind::SeqScan(SeqScanPlan {
                        table_name: String::from(bound_base_table_ref.tbl_name()),
                        table_oid: tbl_info.table_oid(),
                        filter_predicate: None,
                    }),
                })
            }
            BoundTableRef::ExprList(bound_expression_list_ref) => {
                panic!("planner does not support ExprList")
            }
            BoundTableRef::Join(_) => todo!("planner does not support joins yet"),
        }
    }

    fn plan_expression(
        &self,
        expr: BoundExpression,
        children: &[&PlanNode],
    ) -> Result<(Option<String>, PlannedExpression), PlannerError> {
        match expr {
            BoundExpression::Literal(value) => Ok((
                None,
                PlannedExpression {
                    return_type: ExpressionType::from_value(&value),
                    kind: PlannedExpressionKind::ConstantValue(ConstantValueExpression { value }),
                },
            )),
            BoundExpression::Column(column_ref) => {
                let (name, column_value_expr, expr_type) =
                    self.plan_column_ref(column_ref, children)?;
                Ok((
                    name,
                    PlannedExpression {
                        return_type: expr_type,
                        kind: PlannedExpressionKind::ColumnValue(column_value_expr),
                    },
                ))
            }
            BoundExpression::BinaryOp { left, op, right } => {
                let (_, left) = self.plan_expression(*left, children)?;
                let (_, right) = self.plan_expression(*right, children)?;
                let expr = match op {
                    BinaryOperator::Plus => {
                        Self::make_arithmetic_expr(left, right, ArithmeticType::Plus)
                    }
                    BinaryOperator::Minus => {
                        Self::make_arithmetic_expr(left, right, ArithmeticType::Minus)
                    }
                    BinaryOperator::Eq => {
                        Self::make_comparison_expr(left, right, ComparisonType::Eq)
                    }
                    BinaryOperator::NotEq => {
                        Self::make_comparison_expr(left, right, ComparisonType::NotEq)
                    }
                    BinaryOperator::Lt => {
                        Self::make_comparison_expr(left, right, ComparisonType::LessThan)
                    }
                    BinaryOperator::LtEq => {
                        Self::make_comparison_expr(left, right, ComparisonType::LessThanOrEqual)
                    }
                    BinaryOperator::Gt => {
                        Self::make_comparison_expr(left, right, ComparisonType::GreaterThan)
                    }
                    BinaryOperator::GtEq => {
                        Self::make_comparison_expr(left, right, ComparisonType::GreaterThanOrEqual)
                    }
                    BinaryOperator::And => Self::make_logic_expr(left, right, LogicType::And),
                    BinaryOperator::Or => Self::make_logic_expr(left, right, LogicType::Or),
                };

                Ok((None, expr))
            }
            BoundExpression::UnaryOp { expr, op } => {
                let (_, expr) = self.plan_expression(*expr, children)?;
                let expr = match op {
                    UnaryOperator::Not => Self::make_not_expr(expr),
                    UnaryOperator::Neg => Self::make_negate_expr(expr),
                    UnaryOperator::IsNull => {
                        Self::make_null_check_expr(expr, NullCheckType::IsNull)
                    }
                    UnaryOperator::IsNotNull => {
                        Self::make_null_check_expr(expr, NullCheckType::IsNotNull)
                    }
                };

                Ok((None, expr))
            }
        }
    }

    fn make_arithmetic_expr(
        left: PlannedExpression,
        right: PlannedExpression,
        arithmetic_type: ArithmeticType,
    ) -> PlannedExpression {
        let return_type = left.return_type;
        PlannedExpression {
            return_type,
            kind: PlannedExpressionKind::Arithmetic(ArithmeticExpression {
                left: Box::new(left),
                right: Box::new(right),
                arithmetic_type,
            }),
        }
    }

    fn make_comparison_expr(
        left: PlannedExpression,
        right: PlannedExpression,
        comparison_type: ComparisonType,
    ) -> PlannedExpression {
        PlannedExpression {
            return_type: ExpressionType::new_bool(),
            kind: PlannedExpressionKind::Comparison(ComparisonExpression {
                left: Box::new(left),
                right: Box::new(right),
                comparison_type,
            }),
        }
    }

    fn make_logic_expr(
        left: PlannedExpression,
        right: PlannedExpression,
        logic_type: LogicType,
    ) -> PlannedExpression {
        PlannedExpression {
            return_type: ExpressionType::new_bool(),
            kind: PlannedExpressionKind::Logic(LogicExpression {
                left: Box::new(left),
                right: Box::new(right),
                logic_type,
            }),
        }
    }

    fn make_not_expr(expr: PlannedExpression) -> PlannedExpression {
        PlannedExpression {
            return_type: ExpressionType::new_bool(),
            kind: PlannedExpressionKind::Not(NotExpression {
                expr: Box::new(expr),
            }),
        }
    }

    fn make_negate_expr(expr: PlannedExpression) -> PlannedExpression {
        let return_type = expr.return_type;
        PlannedExpression {
            return_type,
            kind: PlannedExpressionKind::Negate(NegateExpression {
                expr: Box::new(expr),
            }),
        }
    }

    fn make_null_check_expr(
        expr: PlannedExpression,
        null_check_type: NullCheckType,
    ) -> PlannedExpression {
        PlannedExpression {
            return_type: ExpressionType::new_bool(),
            kind: PlannedExpressionKind::NullCheck(NullCheckExpression {
                expr: Box::new(expr),
                null_check_type,
            }),
        }
    }

    fn plan_column_ref(
        &self,
        column_ref: ColumnRef,
        children: &[&PlanNode],
    ) -> Result<(Option<String>, ColumnValueExpression, ExpressionType), PlannerError> {
        let col_name = column_ref.to_str();
        match children {
            [child] => match Self::plan_column_ref_from_child(&col_name, child, 0).as_slice() {
                [] => panic!("should not be possible as binder would have caught this?"),
                [(col_expr, expr_type)] => Ok((Some(col_name), col_expr.clone(), *expr_type)),
                _ => Err(PlannerError::AmbiguousColumn(col_name)),
            },
            [left, right] => {
                let mut matched_columns = Self::plan_column_ref_from_child(&col_name, left, 0);
                matched_columns.extend(Self::plan_column_ref_from_child(&col_name, right, 1));

                match matched_columns.as_slice() {
                    [] => panic!("should not be possible as binder would have caught this?"),
                    [(col_expr, expr_type)] => Ok((Some(col_name), col_expr.clone(), *expr_type)),
                    _ => Err(PlannerError::AmbiguousColumn(col_name)),
                }
            }
            _ => panic!("cannot occur"),
        }
    }

    fn plan_column_ref_from_child(
        col_name: &str,
        child: &PlanNode,
        tuple_idx: usize,
    ) -> Vec<(ColumnValueExpression, ExpressionType)> {
        child
            .output_schema()
            .columns()
            .iter()
            .enumerate()
            // Binder normalizes column refs to schema casing and scan schemas use the same qualified names.
            .filter(|(_, col)| col.name() == col_name)
            .map(|(col_idx, col)| {
                (
                    ColumnValueExpression { tuple_idx, col_idx },
                    ExpressionType::from_column(col),
                )
            })
            .collect::<Vec<_>>()
    }
}

#[cfg(test)]
mod tests {
    use expect_test::expect;
    use tempfile::NamedTempFile;

    use crate::{
        buffer::bpm::BufferPoolManager,
        catalog::{column::Column, manager::Catalog, schema::Schema, types::SqlType},
        query::{
            binder::statement::BoundStatement, binder::transformer::Binder, parser::parse_sql,
        },
        storage::disk::disk_manager::DiskManager,
    };

    use super::*;

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

    fn plan_sql(catalog: &Catalog<'_>, sql: &str) -> PlanNode {
        let binder = Binder::new(catalog);
        let planner = Planner::new(catalog);
        let statement = parse_sql(sql).unwrap();
        let bound = binder.bind_statement(statement).unwrap();

        planner.plan_statement(bound).unwrap()
    }

    fn plan_insert_sql(catalog: &Catalog<'_>, sql: &str) -> Result<PlanNode, PlannerError> {
        let binder = Binder::new(catalog);
        let planner = Planner::new(catalog);
        let statement = parse_sql(sql).unwrap();
        let bound = binder.bind_statement(statement).unwrap();
        let BoundStatement::Insert(insert) = bound else {
            panic!("expected insert statement");
        };

        planner.plan_insert(insert)
    }

    fn plan_update_sql(catalog: &Catalog<'_>, sql: &str) -> Result<PlanNode, PlannerError> {
        let binder = Binder::new(catalog);
        let planner = Planner::new(catalog);
        let statement = parse_sql(sql).unwrap();
        let bound = binder.bind_statement(statement).unwrap();
        let BoundStatement::Update(update) = bound else {
            panic!("expected update statement");
        };

        planner.plan_update(update)
    }

    fn plan_delete_sql(catalog: &Catalog<'_>, sql: &str) -> Result<PlanNode, PlannerError> {
        let binder = Binder::new(catalog);
        let planner = Planner::new(catalog);
        let statement = parse_sql(sql).unwrap();
        let bound = binder.bind_statement(statement).unwrap();
        let BoundStatement::Delete(delete) = bound else {
            panic!("expected delete statement");
        };

        planner.plan_delete(delete)
    }

    fn projection_expression(plan: &PlanNode) -> &PlannedExpression {
        let PlanNodeKind::Projection(projection) = &plan.kind else {
            panic!("expected projection plan");
        };

        assert_eq!(projection.expressions.len(), 1);
        &projection.expressions[0]
    }

    fn plan_node_with_columns(columns: &[Column]) -> PlanNode {
        PlanNode {
            output_schema: Schema::new(columns),
            kind: PlanNodeKind::Values(ValuesPlan { rows: vec![] }),
        }
    }

    #[test]
    fn plans_column_ref_from_right_child() {
        let bpm = setup_bpm(3);
        let catalog = Catalog::new(&bpm);
        let planner = Planner::new(&catalog);
        let left =
            plan_node_with_columns(&[Column::new_static("left.id".to_string(), SqlType::Integer)]);
        let right = plan_node_with_columns(&[Column::new_variable(
            "right.name".to_string(),
            SqlType::Varchar,
            32,
        )]);

        let (_, col_expr, expr_type) = planner
            .plan_column_ref(
                ColumnRef::TableQualified {
                    table: "right".to_string(),
                    column: "name".to_string(),
                },
                &[&left, &right],
            )
            .unwrap();

        assert_eq!(col_expr.tuple_idx, 1);
        assert_eq!(col_expr.col_idx, 0);
        assert_eq!(expr_type.sql_type, SqlType::Varchar);
        assert_eq!(expr_type.varchar_size, Some(32));
    }

    #[test]
    fn rejects_column_ref_matching_both_children() {
        let bpm = setup_bpm(3);
        let catalog = Catalog::new(&bpm);
        let planner = Planner::new(&catalog);
        let left =
            plan_node_with_columns(&[Column::new_static("id".to_string(), SqlType::Integer)]);
        let right =
            plan_node_with_columns(&[Column::new_static("id".to_string(), SqlType::Integer)]);

        let err = planner
            .plan_column_ref(
                ColumnRef::Unqualified {
                    column: "id".to_string(),
                },
                &[&left, &right],
            )
            .expect_err("expected ambiguous column error");

        assert!(matches!(err, PlannerError::AmbiguousColumn(col) if col == "id"));
    }

    #[test]
    fn plans_select_column_and_literal_projection() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);

        let plan = plan_sql(&catalog, "select name, 1, id from users");

        expect![[r#"
            PlanNode {
                output_schema: Schema {
                    inlined_storage_size: 13,
                    columns: [
                        Column {
                            name: "users.name",
                            sql_type: Varchar,
                            value_offset: 1,
                            size: Variable(
                                32,
                            ),
                        },
                        Column {
                            name: "__unnamed#1",
                            sql_type: Integer,
                            value_offset: 5,
                            size: Inline(
                                4,
                            ),
                        },
                        Column {
                            name: "users.id",
                            sql_type: Integer,
                            value_offset: 9,
                            size: Inline(
                                4,
                            ),
                        },
                    ],
                    uninlined_columns: [
                        0,
                    ],
                },
                kind: Projection(
                    ProjectionPlan {
                        expressions: [
                            PlannedExpression {
                                return_type: ExpressionType {
                                    sql_type: Varchar,
                                    varchar_size: Some(
                                        32,
                                    ),
                                },
                                kind: ColumnValue(
                                    ColumnValueExpression {
                                        tuple_idx: 0,
                                        col_idx: 1,
                                    },
                                ),
                            },
                            PlannedExpression {
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
                            PlannedExpression {
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
                        ],
                        child: PlanNode {
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
                                    filter_predicate: None,
                                },
                            ),
                        },
                    },
                ),
            }"#]]
        .assert_eq(&format!("{plan:#?}"));
    }

    #[test]
    fn plans_select_arithmetic_projection() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);

        let plan = plan_sql(&catalog, "select id + 1 from users");

        let expr = projection_expression(&plan);
        assert_eq!(expr.return_type.sql_type, SqlType::Integer);
        assert!(matches!(
            expr.kind,
            PlannedExpressionKind::Arithmetic(ArithmeticExpression {
                arithmetic_type: ArithmeticType::Plus,
                ..
            })
        ));
    }

    #[test]
    fn plans_select_subtraction_projection() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);

        let plan = plan_sql(&catalog, "select id - 1 from users");

        let expr = projection_expression(&plan);
        assert_eq!(expr.return_type.sql_type, SqlType::Integer);
        assert!(matches!(
            expr.kind,
            PlannedExpressionKind::Arithmetic(ArithmeticExpression {
                arithmetic_type: ArithmeticType::Minus,
                ..
            })
        ));
    }

    #[test]
    fn plans_select_unary_projection() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);

        let plan = plan_sql(&catalog, "select -id from users");
        let expr = projection_expression(&plan);
        assert_eq!(expr.return_type.sql_type, SqlType::Integer);
        assert!(matches!(expr.kind, PlannedExpressionKind::Negate(_)));

        let plan = plan_sql(&catalog, "select not (id = 1) from users");
        let expr = projection_expression(&plan);
        assert_eq!(expr.return_type.sql_type, SqlType::Boolean);
        assert!(matches!(expr.kind, PlannedExpressionKind::Not(_)));

        let plan = plan_sql(&catalog, "select name is not null from users");
        let expr = projection_expression(&plan);
        assert_eq!(expr.return_type.sql_type, SqlType::Boolean);
        assert!(matches!(
            expr.kind,
            PlannedExpressionKind::NullCheck(NullCheckExpression {
                null_check_type: NullCheckType::IsNotNull,
                ..
            })
        ));
    }

    #[test]
    fn plans_insert_values() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);

        let plan = plan_sql(
            &catalog,
            "insert into users (id, name) values (1, 'alice'), (2, 'bob')",
        );

        expect![[r#"
            PlanNode {
                output_schema: Schema {
                    inlined_storage_size: 5,
                    columns: [
                        Column {
                            name: "__oxtub_internal.insert_rows",
                            sql_type: Integer,
                            value_offset: 1,
                            size: Inline(
                                4,
                            ),
                        },
                    ],
                    uninlined_columns: [],
                },
                kind: Insert(
                    InsertPlan {
                        table_name: "users",
                        table_oid: 0,
                        table_schema: Schema {
                            inlined_storage_size: 9,
                            columns: [
                                Column {
                                    name: "id",
                                    sql_type: Integer,
                                    value_offset: 1,
                                    size: Inline(
                                        4,
                                    ),
                                },
                                Column {
                                    name: "name",
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
                        target_col_idxs: [
                            0,
                            1,
                        ],
                        child: PlanNode {
                            output_schema: Schema {
                                inlined_storage_size: 9,
                                columns: [
                                    Column {
                                        name: "<unnamed>.0",
                                        sql_type: Integer,
                                        value_offset: 1,
                                        size: Inline(
                                            4,
                                        ),
                                    },
                                    Column {
                                        name: "<unnamed>.1",
                                        sql_type: Varchar,
                                        value_offset: 5,
                                        size: Variable(
                                            5,
                                        ),
                                    },
                                ],
                                uninlined_columns: [
                                    1,
                                ],
                            },
                            kind: Values(
                                ValuesPlan {
                                    rows: [
                                        [
                                            PlannedExpression {
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
                                            PlannedExpression {
                                                return_type: ExpressionType {
                                                    sql_type: Varchar,
                                                    varchar_size: Some(
                                                        5,
                                                    ),
                                                },
                                                kind: ConstantValue(
                                                    ConstantValueExpression {
                                                        value: Varchar(
                                                            "alice",
                                                        ),
                                                    },
                                                ),
                                            },
                                        ],
                                        [
                                            PlannedExpression {
                                                return_type: ExpressionType {
                                                    sql_type: Integer,
                                                    varchar_size: None,
                                                },
                                                kind: ConstantValue(
                                                    ConstantValueExpression {
                                                        value: Integer(
                                                            2,
                                                        ),
                                                    },
                                                ),
                                            },
                                            PlannedExpression {
                                                return_type: ExpressionType {
                                                    sql_type: Varchar,
                                                    varchar_size: Some(
                                                        3,
                                                    ),
                                                },
                                                kind: ConstantValue(
                                                    ConstantValueExpression {
                                                        value: Varchar(
                                                            "bob",
                                                        ),
                                                    },
                                                ),
                                            },
                                        ],
                                    ],
                                },
                            ),
                        },
                    },
                ),
            }"#]]
        .assert_eq(&format!("{plan:#?}"));
    }

    #[test]
    fn plans_update_values_without_where_clause() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);

        let plan = plan_sql(&catalog, "update users set name = 'bob'");

        expect![[r#"
            PlanNode {
                output_schema: Schema {
                    inlined_storage_size: 5,
                    columns: [
                        Column {
                            name: "__oxtub_internal.update_rows",
                            sql_type: Integer,
                            value_offset: 1,
                            size: Inline(
                                4,
                            ),
                        },
                    ],
                    uninlined_columns: [],
                },
                kind: Update(
                    UpdatePlan {
                        table_name: "users",
                        table_oid: 0,
                        table_schema: Schema {
                            inlined_storage_size: 9,
                            columns: [
                                Column {
                                    name: "id",
                                    sql_type: Integer,
                                    value_offset: 1,
                                    size: Inline(
                                        4,
                                    ),
                                },
                                Column {
                                    name: "name",
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
                        expressions: [
                            PlannedExpression {
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
                            PlannedExpression {
                                return_type: ExpressionType {
                                    sql_type: Varchar,
                                    varchar_size: Some(
                                        3,
                                    ),
                                },
                                kind: ConstantValue(
                                    ConstantValueExpression {
                                        value: Varchar(
                                            "bob",
                                        ),
                                    },
                                ),
                            },
                        ],
                        child: PlanNode {
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
                                    filter_predicate: None,
                                },
                            ),
                        },
                    },
                ),
            }"#]]
        .assert_eq(&format!("{plan:#?}"));
    }

    #[test]
    fn rejects_update_when_assignment_type_does_not_match_column() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);

        let err = plan_update_sql(&catalog, "update users set id = 'bad'")
            .expect_err("expected update schema mismatch");

        assert!(matches!(err, PlannerError::UpdateSchemaMismatch));
    }

    #[test]
    fn plans_delete_without_where_clause() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);

        let plan = plan_sql(&catalog, "delete from users");

        expect![[r#"
            PlanNode {
                output_schema: Schema {
                    inlined_storage_size: 5,
                    columns: [
                        Column {
                            name: "__oxtub_internal.delete_rows",
                            sql_type: Integer,
                            value_offset: 1,
                            size: Inline(
                                4,
                            ),
                        },
                    ],
                    uninlined_columns: [],
                },
                kind: Delete(
                    DeletePlan {
                        table_oid: 0,
                        child: PlanNode {
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
                                    filter_predicate: None,
                                },
                            ),
                        },
                    },
                ),
            }"#]]
        .assert_eq(&format!("{plan:#?}"));
    }

    #[test]
    fn plans_delete_with_where_clause() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);

        let plan = plan_delete_sql(&catalog, "delete from users where id = 1").unwrap();

        let PlanNodeKind::Delete(delete) = &plan.kind else {
            panic!("expected delete plan");
        };
        let PlanNodeKind::Filter(filter) = &delete.child.kind else {
            panic!("expected filter below delete");
        };
        assert!(matches!(
            filter.predicate.kind,
            PlannedExpressionKind::Comparison(_)
        ));
    }

    #[test]
    #[ignore = "TODO: validate every row in VALUES planning"]
    fn rejects_multi_row_insert_when_later_row_type_differs_from_first_row() {
        let bpm = setup_bpm(3);
        let mut catalog = Catalog::new(&bpm);
        create_users_table(&mut catalog);

        // TODO: fix VALUES planning to validate every row, not just infer schema from the first row.
        let err = plan_insert_sql(&catalog, "insert into users (id) values (1), ('bad')")
            .expect_err("expected insert schema mismatch");

        assert!(matches!(err, PlannerError::InsertSchemaMismatch));
    }
}
