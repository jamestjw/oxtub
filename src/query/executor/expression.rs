use crate::{
    query::{
        executor::{engine::ExecutorRow, error::ExecutionError},
        planner::expression::PlannedExpression,
    },
    types::value::Value,
};

pub fn evaluate_expression(
    _expr: &PlannedExpression,
    _row: Option<&ExecutorRow>,
) -> Result<Value, ExecutionError> {
    todo!("evaluate expression")
}
