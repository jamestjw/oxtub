use crate::{
    catalog::types::SqlType,
    query::{
        executor::{engine::ExecutorRow, error::ExecutionError},
        planner::expression::{
            ColumnValueExpression, ConstantValueExpression, LogicExpression, LogicType,
            NegateExpression, NullCheckExpression, NullCheckType, PlannedExpression,
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
        PlannedExpressionKind::ColumnValue(ColumnValueExpression { tuple_idx, col_idx }) => {
            assert_eq!(*tuple_idx, 0);
            // TODO: we should handle joins separately
            Ok(row.values[*col_idx].clone())
        }
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
        PlannedExpressionKind::Negate(NegateExpression { expr }) => {
            match evaluate_expression(expr, row)? {
                Value::SmallInt(i) => i
                    .checked_neg()
                    .map(Value::SmallInt)
                    .ok_or(ExecutionError::NumericOutOfRange),
                Value::Integer(i) => i
                    .checked_neg()
                    .map(Value::Integer)
                    .ok_or(ExecutionError::NumericOutOfRange),
                Value::BigInt(i) => i
                    .checked_neg()
                    .map(Value::BigInt)
                    .ok_or(ExecutionError::NumericOutOfRange),
                Value::Decimal(f) => Ok(Value::Decimal(-f)),
                v @ Value::Null(
                    SqlType::BigInt | SqlType::Decimal | SqlType::Integer | SqlType::SmallInt,
                ) => Ok(v),
                v => Err(ExecutionError::ExpectedNumeric(v)),
            }
        }
        PlannedExpressionKind::NullCheck(NullCheckExpression {
            expr,
            null_check_type,
        }) => match (null_check_type, evaluate_expression(expr, row)?) {
            (NullCheckType::IsNull, Value::Null(_)) => Ok(Value::Boolean(true)),
            (NullCheckType::IsNull, _) => Ok(Value::Boolean(false)),
            (NullCheckType::IsNotNull, Value::Null(_)) => Ok(Value::Boolean(false)),
            (NullCheckType::IsNotNull, _) => Ok(Value::Boolean(true)),
        },
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
