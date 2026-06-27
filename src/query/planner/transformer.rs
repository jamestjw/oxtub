use crate::{
    catalog::{manager::Catalog, schema::Schema},
    query::{
        binder::{
            expression::BoundExpression,
            statement::{BoundSelect, BoundStatement},
            table_ref::{BoundExpressionListRef, TableRef},
        },
        planner::{
            error::PlannerError,
            expression::{
                ColumnValueExpression, ConstantValueExpression, ExpressionType, PlannedExpression,
                PlannedExpressionKind,
            },
            plan::{FilterPlan, PlanNode, PlanNodeKind, ProjectionPlan, SeqScanPlan, ValuesPlan},
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
                        let (_, expr) = self.plan_expression(expr, vec![])?;
                        Ok(expr)
                    })
                    .collect::<Result<Vec<_>, PlannerError>>()
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Make schema from first row since all rows should have the same schema
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
                    return_type: ExpressionType::from_value(&value),
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
                                return_type: ExpressionType::from_column(col),
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

#[cfg(test)]
mod tests {
    use expect_test::expect;
    use tempfile::NamedTempFile;

    use crate::{
        buffer::bpm::BufferPoolManager,
        catalog::{column::Column, manager::Catalog, schema::Schema, types::SqlType},
        query::{binder::transformer::Binder, parser::parse_sql},
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
                                },
                            ),
                        },
                    },
                ),
            }"#]]
        .assert_eq(&format!("{plan:#?}"));
    }
}
