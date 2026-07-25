use crate::{catalog::schema::Schema, query::executor::ExecutorRow, types::value::Value};

pub fn build_join_tuple(
    output_schema: &Schema,
    left_tuple: &ExecutorRow,
    right_tuple: Option<&ExecutorRow>,
) -> ExecutorRow {
    let mut values = Vec::with_capacity(output_schema.num_columns());
    values.extend_from_slice(&left_tuple.values);

    match right_tuple {
        Some(right_tuple) => values.extend_from_slice(&right_tuple.values),
        None => {
            let right_null_values = output_schema.columns()[values.len()..]
                .iter()
                .map(|col| Value::Null(col.sql_type()))
                .collect::<Vec<_>>();
            values.extend(right_null_values);
        }
    }

    ExecutorRow { rid: None, values }
}
