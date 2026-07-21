use std::fmt::Write;

use crate::{
    query::{
        planner::{
            expression::{
                ArithmeticType, ComparisonType, LogicType, NullCheckType, PlannedExpression,
                PlannedExpressionKind,
            },
            plan::{PlanNode, PlanNodeKind},
        },
        table_ref::JoinType,
    },
    types::value::Value,
};

pub fn format_plan(plan: &PlanNode) -> String {
    let mut out = String::new();
    format_plan_node(plan, 0, &mut out);
    // Keep multiline plans easy to assert in SLT files by omitting the final newline.
    out.trim_end_matches('\n').to_string()
}

fn format_plan_node(plan: &PlanNode, indent: usize, out: &mut String) {
    out.push_str(&"  ".repeat(indent));

    match &plan.kind {
        PlanNodeKind::SeqScan(seq_scan) => {
            let _ = write!(out, "SeqScan table={}", seq_scan.table_name);
            if let Some(predicate) = &seq_scan.filter_predicate {
                let _ = write!(out, " filter={}", format_expr(predicate));
            }
            out.push('\n');
        }
        PlanNodeKind::Filter(filter) => {
            let _ = writeln!(out, "Filter predicate={}", format_expr(&filter.predicate));
            format_plan_node(&filter.child, indent + 1, out);
        }
        PlanNodeKind::Projection(projection) => {
            let expressions = projection
                .expressions
                .iter()
                .map(format_expr)
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(out, "Projection exprs=[{expressions}]");
            format_plan_node(&projection.child, indent + 1, out);
        }
        PlanNodeKind::Values(values) => {
            let _ = writeln!(out, "Values rows={}", values.rows.len());
        }
        PlanNodeKind::Insert(insert) => {
            let _ = writeln!(out, "Insert table={}", insert.table_name);
            format_plan_node(&insert.child, indent + 1, out);
        }
        PlanNodeKind::CreateTable(create_table) => {
            let _ = writeln!(out, "CreateTable table={}", create_table.name);
        }
        PlanNodeKind::Update(update) => {
            let expressions = update
                .expressions
                .iter()
                .map(format_expr)
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(
                out,
                "Update table={} exprs=[{expressions}]",
                update.table_name
            );
            format_plan_node(&update.child, indent + 1, out);
        }
        PlanNodeKind::Delete(delete) => {
            let _ = writeln!(out, "Delete table_oid={}", delete.table_oid);
            format_plan_node(&delete.child, indent + 1, out);
        }
        PlanNodeKind::NestedLoopJoin(nlj) => {
            let _ = write!(
                out,
                "NestedLoopJoin type={}",
                format_join_type(nlj.join_type)
            );
            if let Some(predicate) = &nlj.predicate {
                let _ = write!(out, " predicate={}", format_expr(predicate));
            }
            out.push('\n');
            format_plan_node(&nlj.left, indent + 1, out);
            format_plan_node(&nlj.right, indent + 1, out);
        }
    }
}

fn format_expr(expr: &PlannedExpression) -> String {
    match &expr.kind {
        PlannedExpressionKind::ColumnValue(column) => {
            format!("#{}.{}", column.tuple_idx, column.col_idx)
        }
        PlannedExpressionKind::ConstantValue(constant) => format_value(&constant.value),
        PlannedExpressionKind::Comparison(comparison) => format!(
            "({} {} {})",
            format_expr(&comparison.left),
            format_comparison_type(&comparison.comparison_type),
            format_expr(&comparison.right)
        ),
        PlannedExpressionKind::Arithmetic(arithmetic) => format!(
            "({} {} {})",
            format_expr(&arithmetic.left),
            format_arithmetic_type(&arithmetic.arithmetic_type),
            format_expr(&arithmetic.right)
        ),
        PlannedExpressionKind::Logic(logic) => format!(
            "({} {} {})",
            format_expr(&logic.left),
            format_logic_type(&logic.logic_type),
            format_expr(&logic.right)
        ),
        PlannedExpressionKind::Not(not) => format!("NOT {}", format_expr(&not.expr)),
        PlannedExpressionKind::Negate(negate) => format!("-{}", format_expr(&negate.expr)),
        PlannedExpressionKind::NullCheck(null_check) => format!(
            "{} {}",
            format_expr(&null_check.expr),
            format_null_check_type(&null_check.null_check_type)
        ),
    }
}

fn format_value(value: &Value) -> String {
    match value {
        Value::Boolean(value) => value.to_string(),
        Value::SmallInt(value) => value.to_string(),
        Value::Integer(value) => value.to_string(),
        Value::BigInt(value) => value.to_string(),
        Value::Decimal(value) => value.to_string(),
        Value::Varchar(value) => format!("'{value}'"),
        Value::Null(_) => "NULL".to_string(),
    }
}

fn format_comparison_type(comparison_type: &ComparisonType) -> &'static str {
    match comparison_type {
        ComparisonType::Eq => "=",
        ComparisonType::NotEq => "!=",
        ComparisonType::LessThan => "<",
        ComparisonType::LessThanOrEqual => "<=",
        ComparisonType::GreaterThan => ">",
        ComparisonType::GreaterThanOrEqual => ">=",
    }
}

fn format_arithmetic_type(arithmetic_type: &ArithmeticType) -> &'static str {
    match arithmetic_type {
        ArithmeticType::Plus => "+",
        ArithmeticType::Minus => "-",
    }
}

fn format_logic_type(logic_type: &LogicType) -> &'static str {
    match logic_type {
        LogicType::And => "AND",
        LogicType::Or => "OR",
    }
}

fn format_null_check_type(null_check_type: &NullCheckType) -> &'static str {
    match null_check_type {
        NullCheckType::IsNull => "IS NULL",
        NullCheckType::IsNotNull => "IS NOT NULL",
    }
}

fn format_join_type(join_type: JoinType) -> &'static str {
    match join_type {
        JoinType::Inner => "inner",
        JoinType::Left => "left",
        JoinType::Cross => "cross",
    }
}
