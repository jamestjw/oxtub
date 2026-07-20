use crate::{
    catalog::{schema::Schema, types::SqlType},
    query::{
        executor::{engine::ExecutorRow, error::ExecutionError},
        planner::expression::{
            ArithmeticExpression, ArithmeticType, ColumnValueExpression, ComparisonExpression,
            ComparisonType, ConstantValueExpression, LogicExpression, LogicType, NegateExpression,
            NullCheckExpression, NullCheckType, PlannedExpression, PlannedExpressionKind,
        },
    },
    storage::table::tuple::Tuple,
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
        PlannedExpressionKind::Comparison(ComparisonExpression {
            left,
            comparison_type,
            right,
        }) => {
            let left_value = evaluate_expression(left, row)?;
            let right_value = evaluate_expression(right, row)?;
            eval_comparison(left_value, right_value, comparison_type)
        }
        PlannedExpressionKind::Arithmetic(ArithmeticExpression {
            left,
            arithmetic_type,
            right,
        }) => {
            let left_value = evaluate_expression(left, row)?;
            let right_value = evaluate_expression(right, row)?;

            eval_arithmetic(arithmetic_type, left_value, right_value)
        }
        PlannedExpressionKind::Logic(LogicExpression {
            left,
            logic_type,
            right,
        }) => {
            let left_value = expect_bool(evaluate_expression(left, row)?)?;
            let right_value = expect_bool(evaluate_expression(right, row)?)?;

            eval_logic(logic_type, left_value, right_value)
        }
        PlannedExpressionKind::Not(not_expression) => {
            eval_not(evaluate_expression(&not_expression.expr, row)?)
        }
        PlannedExpressionKind::Negate(NegateExpression { expr }) => {
            eval_negate(evaluate_expression(expr, row)?)
        }
        PlannedExpressionKind::NullCheck(NullCheckExpression {
            expr,
            null_check_type,
        }) => Ok(eval_null_check(
            null_check_type,
            evaluate_expression(expr, row)?,
        )),
    }
}

pub fn evaluate_join_expression(
    expr: &PlannedExpression,
    left_row: &ExecutorRow,
    right_row: &ExecutorRow,
) -> Result<Value, ExecutionError> {
    match &expr.kind {
        PlannedExpressionKind::ColumnValue(ColumnValueExpression { tuple_idx, col_idx }) => {
            match tuple_idx {
                0 => Ok(left_row.values[*col_idx].clone()),
                1 => Ok(right_row.values[*col_idx].clone()),
                _ => todo!("don't support more than two joined tables"),
            }
        }
        PlannedExpressionKind::ConstantValue(ConstantValueExpression { value }) => {
            Ok(value.clone())
        }
        PlannedExpressionKind::Comparison(ComparisonExpression {
            left,
            comparison_type,
            right,
        }) => {
            let left_value = evaluate_join_expression(left, left_row, right_row)?;
            let right_value = evaluate_join_expression(right, left_row, right_row)?;
            eval_comparison(left_value, right_value, comparison_type)
        }
        PlannedExpressionKind::Arithmetic(ArithmeticExpression {
            left,
            arithmetic_type,
            right,
        }) => {
            let left_value = evaluate_join_expression(left, left_row, right_row)?;
            let right_value = evaluate_join_expression(right, left_row, right_row)?;

            eval_arithmetic(arithmetic_type, left_value, right_value)
        }
        PlannedExpressionKind::Logic(LogicExpression {
            left,
            logic_type,
            right,
        }) => {
            let left_value = expect_bool(evaluate_join_expression(left, left_row, right_row)?)?;
            let right_value = expect_bool(evaluate_join_expression(right, left_row, right_row)?)?;

            eval_logic(logic_type, left_value, right_value)
        }

        PlannedExpressionKind::Not(not_expression) => eval_not(evaluate_join_expression(
            &not_expression.expr,
            left_row,
            right_row,
        )?),
        PlannedExpressionKind::Negate(NegateExpression { expr }) => {
            eval_negate(evaluate_join_expression(expr, left_row, right_row)?)
        }
        PlannedExpressionKind::NullCheck(NullCheckExpression {
            expr,
            null_check_type,
        }) => Ok(eval_null_check(
            null_check_type,
            evaluate_join_expression(expr, left_row, right_row)?,
        )),
    }
}

fn eval_not(value: Value) -> Result<Value, ExecutionError> {
    match expect_bool(value)? {
        CmpBool::True => Ok(Value::Boolean(false)),
        CmpBool::False => Ok(Value::Boolean(true)),
        CmpBool::Null => Ok(Value::Null(SqlType::Boolean)),
    }
}

fn eval_negate(value: Value) -> Result<Value, ExecutionError> {
    match value {
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

fn eval_null_check(null_check_type: &NullCheckType, value: Value) -> Value {
    let is_null = value.is_null();
    match null_check_type {
        NullCheckType::IsNull => Value::Boolean(is_null),
        NullCheckType::IsNotNull => Value::Boolean(!is_null),
    }
}

pub fn evaluate_expression_on_tuple(
    expr: &PlannedExpression,
    tuple: &Tuple,
    schema: &Schema,
) -> Result<Value, ExecutionError> {
    let row = ExecutorRow {
        rid: None,
        values: tuple.get_values(schema),
    };

    evaluate_expression(expr, &row)
}

pub fn filter_keep_row(
    predicate: &PlannedExpression,
    row: &ExecutorRow,
) -> Result<bool, ExecutionError> {
    match evaluate_expression(predicate, row)? {
        Value::Boolean(true) => Ok(true),
        Value::Boolean(false) | Value::Null(_) => Ok(false),
        value => Err(ExecutionError::ExpectedBoolean(value)),
    }
}

pub fn filter_join_row(
    predicate: &PlannedExpression,
    left: &ExecutorRow,
    right: &ExecutorRow,
) -> Result<bool, ExecutionError> {
    match evaluate_join_expression(predicate, left, right)? {
        Value::Boolean(true) => Ok(true),
        Value::Boolean(false) | Value::Null(_) => Ok(false),
        value => Err(ExecutionError::ExpectedBoolean(value)),
    }
}

enum CmpBool {
    True,
    False,
    Null,
}

enum CmpNumeric {
    SmallInt(i16),
    Integer(i32),
    BigInt(i64),
    Decimal(f64),
}

fn eval_comparison(
    left: Value,
    right: Value,
    comparison_type: &ComparisonType,
) -> Result<Value, ExecutionError> {
    if can_compare_as_numeric(&left, &right) {
        return eval_numeric_comparison(left, right, comparison_type);
    }

    if left.is_null() || right.is_null() {
        if left.sql_type() == right.sql_type() {
            return Ok(Value::Null(SqlType::Boolean));
        }
        return Err(ExecutionError::ComparisonTypeMismatch(left, right));
    }

    let res = match (left, right) {
        (Value::Boolean(left), Value::Boolean(right)) => {
            eval_ord_comparison(left, right, comparison_type)
        }
        (Value::Varchar(left), Value::Varchar(right)) => {
            eval_ord_comparison(left, right, comparison_type)
        }
        (left, right) => return Err(ExecutionError::ComparisonTypeMismatch(left, right)),
    };

    Ok(Value::Boolean(res))
}

fn eval_numeric_comparison(
    left: Value,
    right: Value,
    comparison_type: &ComparisonType,
) -> Result<Value, ExecutionError> {
    let result_type = numeric_result_type(&left, &right)?;

    if left.is_null() || right.is_null() {
        return Ok(Value::Null(SqlType::Boolean));
    }

    let left = cast_numeric(left, result_type).unwrap();
    let right = cast_numeric(right, result_type).unwrap();

    let res = match (left, right) {
        (CmpNumeric::SmallInt(left), CmpNumeric::SmallInt(right)) => {
            eval_ord_comparison(left, right, comparison_type)
        }
        (CmpNumeric::Integer(left), CmpNumeric::Integer(right)) => {
            eval_ord_comparison(left, right, comparison_type)
        }
        (CmpNumeric::BigInt(left), CmpNumeric::BigInt(right)) => {
            eval_ord_comparison(left, right, comparison_type)
        }
        (CmpNumeric::Decimal(left), CmpNumeric::Decimal(right)) => {
            eval_ord_comparison(left, right, comparison_type)
        }
        _ => unreachable!("numeric operands should be widened to the same type"),
    };

    Ok(Value::Boolean(res))
}

fn eval_arithmetic(
    arithmetic_type: &ArithmeticType,
    left_value: Value,
    right_value: Value,
) -> Result<Value, ExecutionError> {
    match arithmetic_type {
        ArithmeticType::Plus => eval_numeric_arithmetic(left_value, right_value, eval_add),
        ArithmeticType::Minus => eval_numeric_arithmetic(left_value, right_value, eval_sub),
    }
}

fn eval_ord_comparison<T: PartialEq + PartialOrd>(
    left: T,
    right: T,
    comparison_type: &ComparisonType,
) -> bool {
    match comparison_type {
        ComparisonType::Eq => left == right,
        ComparisonType::NotEq => left != right,
        ComparisonType::LessThan => left < right,
        ComparisonType::LessThanOrEqual => left <= right,
        ComparisonType::GreaterThan => left > right,
        ComparisonType::GreaterThanOrEqual => left >= right,
    }
}

fn can_compare_as_numeric(left: &Value, right: &Value) -> bool {
    is_numeric_type(left.sql_type()) && is_numeric_type(right.sql_type())
}

fn is_numeric_type(sql_type: SqlType) -> bool {
    matches!(
        sql_type,
        SqlType::SmallInt | SqlType::Integer | SqlType::BigInt | SqlType::Decimal
    )
}

fn eval_numeric_arithmetic(
    left: Value,
    right: Value,

    op: fn(CmpNumeric, CmpNumeric) -> Result<Value, ExecutionError>,
) -> Result<Value, ExecutionError> {
    let result_type = numeric_result_type(&left, &right)?;

    if left.is_null() || right.is_null() {
        return Ok(Value::Null(result_type));
    }

    let left = cast_numeric(left, result_type).unwrap();
    let right = cast_numeric(right, result_type).unwrap();
    op(left, right)
}

fn numeric_result_type(left: &Value, right: &Value) -> Result<SqlType, ExecutionError> {
    let left_type = expect_numeric_type(left)?;
    let right_type = expect_numeric_type(right)?;

    match (left_type, right_type) {
        (SqlType::Decimal, _) | (_, SqlType::Decimal) => Ok(SqlType::Decimal),
        (SqlType::BigInt, _) | (_, SqlType::BigInt) => Ok(SqlType::BigInt),
        (SqlType::Integer, _) | (_, SqlType::Integer) => Ok(SqlType::Integer),
        (SqlType::SmallInt, SqlType::SmallInt) => Ok(SqlType::SmallInt),
        _ => unreachable!("numeric types should have been validated"),
    }
}

fn expect_numeric_type(value: &Value) -> Result<SqlType, ExecutionError> {
    let sql_type = value.sql_type();
    match sql_type {
        SqlType::SmallInt | SqlType::Integer | SqlType::BigInt | SqlType::Decimal => Ok(sql_type),
        _ => Err(ExecutionError::ExpectedNumeric(value.clone())),
    }
}

fn cast_numeric(value: Value, target_type: SqlType) -> Result<CmpNumeric, ExecutionError> {
    match (value, target_type) {
        (Value::SmallInt(i), SqlType::SmallInt) => Ok(CmpNumeric::SmallInt(i)),
        (Value::SmallInt(i), SqlType::Integer) => Ok(CmpNumeric::Integer(i.into())),
        (Value::SmallInt(i), SqlType::BigInt) => Ok(CmpNumeric::BigInt(i.into())),
        (Value::SmallInt(i), SqlType::Decimal) => Ok(CmpNumeric::Decimal(i.into())),
        (Value::Integer(i), SqlType::Integer) => Ok(CmpNumeric::Integer(i)),
        (Value::Integer(i), SqlType::BigInt) => Ok(CmpNumeric::BigInt(i.into())),
        (Value::Integer(i), SqlType::Decimal) => Ok(CmpNumeric::Decimal(i.into())),
        (Value::BigInt(i), SqlType::BigInt) => Ok(CmpNumeric::BigInt(i)),
        (Value::BigInt(i), SqlType::Decimal) => Ok(CmpNumeric::Decimal(i as f64)),
        (Value::Decimal(f), SqlType::Decimal) => Ok(CmpNumeric::Decimal(f)),
        (value, _) => Err(ExecutionError::ExpectedNumeric(value)),
    }
}

fn eval_add(left: CmpNumeric, right: CmpNumeric) -> Result<Value, ExecutionError> {
    match (left, right) {
        (CmpNumeric::SmallInt(left), CmpNumeric::SmallInt(right)) => left
            .checked_add(right)
            .map(Value::SmallInt)
            .ok_or(ExecutionError::NumericOutOfRange),
        (CmpNumeric::Integer(left), CmpNumeric::Integer(right)) => left
            .checked_add(right)
            .map(Value::Integer)
            .ok_or(ExecutionError::NumericOutOfRange),
        (CmpNumeric::BigInt(left), CmpNumeric::BigInt(right)) => left
            .checked_add(right)
            .map(Value::BigInt)
            .ok_or(ExecutionError::NumericOutOfRange),
        (CmpNumeric::Decimal(left), CmpNumeric::Decimal(right)) => Ok(Value::Decimal(left + right)),
        _ => unreachable!("numeric operands should be widened to the same type"),
    }
}

fn eval_sub(left: CmpNumeric, right: CmpNumeric) -> Result<Value, ExecutionError> {
    match (left, right) {
        (CmpNumeric::SmallInt(left), CmpNumeric::SmallInt(right)) => left
            .checked_sub(right)
            .map(Value::SmallInt)
            .ok_or(ExecutionError::NumericOutOfRange),
        (CmpNumeric::Integer(left), CmpNumeric::Integer(right)) => left
            .checked_sub(right)
            .map(Value::Integer)
            .ok_or(ExecutionError::NumericOutOfRange),
        (CmpNumeric::BigInt(left), CmpNumeric::BigInt(right)) => left
            .checked_sub(right)
            .map(Value::BigInt)
            .ok_or(ExecutionError::NumericOutOfRange),
        (CmpNumeric::Decimal(left), CmpNumeric::Decimal(right)) => Ok(Value::Decimal(left - right)),
        _ => unreachable!("numeric operands should be widened to the same type"),
    }
}

fn expect_bool(value: Value) -> Result<CmpBool, ExecutionError> {
    match value {
        Value::Boolean(true) => Ok(CmpBool::True),
        Value::Boolean(false) => Ok(CmpBool::False),
        Value::Null(_) => Ok(CmpBool::Null),
        value => Err(ExecutionError::ExpectedBoolean(value)),
    }
}

fn eval_logic(
    logic_type: &LogicType,
    left_value: CmpBool,
    right_value: CmpBool,
) -> Result<Value, ExecutionError> {
    match logic_type {
        LogicType::And => eval_and(left_value, right_value),
        LogicType::Or => eval_or(left_value, right_value),
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
