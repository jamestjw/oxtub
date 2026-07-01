use crate::{
    catalog::types::SqlType,
    query::{
        executor::{engine::ExecutorRow, error::ExecutionError},
        planner::expression::{
            ConstantValueExpression, LogicExpression, LogicType, PlannedExpression,
            PlannedExpressionKind,
        },
    },
    types::value::Value,
};

pub fn evaluate_expression(
    expr: &PlannedExpression,
    row: &ExecutorRow,
) -> Result<Value, ExecutionError> {
    match &expr.kind {
        PlannedExpressionKind::ColumnValue(column_value_expression) => todo!(),
        PlannedExpressionKind::ConstantValue(ConstantValueExpression { value }) => {
            Ok(value.clone())
        }
        PlannedExpressionKind::Comparison(comparison_expression) => todo!(),
        PlannedExpressionKind::Arithmetic(arithmetic_expression) => todo!(),
        PlannedExpressionKind::Logic(LogicExpression {
            left,
            logic_type,
            right,
        }) => {
            let left_value = expect_bool(evaluate_expression(left, row)?)?;
            let right_value = expect_bool(evaluate_expression(right, row)?)?;

            match logic_type {
                LogicType::And => eval_and(left_value, right_value),
                LogicType::Or => eval_or(left_value, right_value),
            }
        }
        PlannedExpressionKind::Not(not_expression) => {
            match expect_bool(evaluate_expression(&not_expression.expr, row)?)? {
                CmpBool::True => Ok(Value::Boolean(false)),
                CmpBool::False => Ok(Value::Boolean(true)),
                CmpBool::Null => Ok(Value::Null(SqlType::Boolean)),
            }
        }
        PlannedExpressionKind::Negate(negate_expression) => todo!(),
        PlannedExpressionKind::NullCheck(null_check_expression) => todo!(),
    }
}

enum CmpBool {
    True,
    False,
    Null,
}

fn expect_bool(value: Value) -> Result<CmpBool, ExecutionError> {
    match value {
        Value::Boolean(true) => Ok(CmpBool::True),
        Value::Boolean(false) => Ok(CmpBool::False),
        Value::Null(_) => Ok(CmpBool::Null),
        value => Err(ExecutionError::ExpectedBoolean(value)),
    }
}

fn eval_and(left: CmpBool, right: CmpBool) -> Result<Value, ExecutionError> {
    match (left, right) {
        (CmpBool::False, _) | (_, CmpBool::False) => Ok(Value::Boolean(false)),
        (CmpBool::True, CmpBool::True) => Ok(Value::Boolean(true)),
        (CmpBool::Null, _) | (_, CmpBool::Null) => Ok(Value::Null(SqlType::Boolean)),
    }
}

fn eval_or(left: CmpBool, right: CmpBool) -> Result<Value, ExecutionError> {
    match (left, right) {
        (CmpBool::True, _) | (_, CmpBool::True) => Ok(Value::Boolean(true)),
        (CmpBool::False, CmpBool::False) => Ok(Value::Boolean(false)),
        (CmpBool::Null, _) | (_, CmpBool::Null) => Ok(Value::Null(SqlType::Boolean)),
    }
}
