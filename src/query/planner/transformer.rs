use crate::{
    catalog::{column::Column, manager::Catalog, schema::Schema, types::SqlType},
    query::{
        binder::{
            expression::{BoundExpression, ColumnRef},
            statement::{BoundDelete, BoundInsert, BoundSelect, BoundStatement, BoundUpdate},
            table_ref::{BoundExpressionListRef, TableRef},
        },
        planner::{
            error::PlannerError,
            expression::{
                ColumnValueExpression, ConstantValueExpression, ExpressionType, PlannedExpression,
                PlannedExpressionKind,
            },
            plan::{
                FilterPlan, InsertPlan, PlanNode, PlanNodeKind, ProjectionPlan, SeqScanPlan,
                UpdatePlan, ValuesPlan,
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

        let planned_table = self.plan_table_ref(TableRef::BaseTable(bound_update.table))?;
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
        todo!()
    }

    fn plan_insert(&self, stmt: BoundInsert) -> Result<PlanNode, PlannerError> {
        let planned_expr_list = self.plan_bound_expression_list(stmt.bound_exprs)?;
        let insert_columns = stmt.columns;
        let table_schema = stmt.table.schema();
        let child_schema = planned_expr_list.output_schema();

        if insert_columns.len() != child_schema.columns().len() {
            return Err(PlannerError::InsertSchemaMismatch);
        }

        for (insert_col, child_col) in insert_columns.iter().zip(child_schema.columns()) {
            let insert_col_name = match insert_col {
                ColumnRef::Unqualified { column } => column,
                ColumnRef::TableQualified { column, .. } => column,
            };

            let target_col = table_schema
                .columns()
                .iter()
                .find(|col| col.name() == insert_col_name)
                .expect("binder should have resolved insert columns");

            if target_col.sql_type() != child_col.sql_type() {
                return Err(PlannerError::InsertSchemaMismatch);
            }
            if target_col.sql_type().is_varlen()
                && child_col.declared_size() > target_col.declared_size()
            {
                return Err(PlannerError::InsertSchemaMismatch);
            }
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
                columns: insert_columns,
                child: Box::new(planned_expr_list),
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
            BoundExpression::BinaryOp { left, op, right } => todo!(),
            BoundExpression::UnaryOp { expr, op } => todo!(),
        }
    }

    fn plan_column_ref(
        &self,
        column_ref: ColumnRef,
        children: &[&PlanNode],
    ) -> Result<(Option<String>, ColumnValueExpression, ExpressionType), PlannerError> {
        match children {
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
                        ColumnValueExpression {
                            tuple_idx: 0,
                            col_idx: idx,
                        },
                        ExpressionType::from_column(col),
                    )),
                    _ => Err(PlannerError::AmbiguousColumn(col_name)),
                }
            }
            [_left, _right] => todo!("binder doesnt support joins yet!"),
            _ => panic!("cannot occur"),
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
                        columns: [
                            TableQualified {
                                table: "users",
                                column: "id",
                            },
                            TableQualified {
                                table: "users",
                                column: "name",
                            },
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
