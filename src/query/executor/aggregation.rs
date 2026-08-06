use std::{
    collections::{HashMap, hash_map::Entry},
    hash::{Hash, Hasher},
};

use crate::{
    catalog::types::SqlType,
    query::{
        binder::expression::AggregationType,
        executor::{
            error::ExecutionError,
            expression::{eval_arithmetic, eval_comparison_is_true},
        },
        planner::expression::{ArithmeticType, ComparisonType, PlannedExpression},
    },
    types::value::Value,
};

struct AggregateKey {
    group_bys: Vec<Value>,
}

impl Hash for AggregateKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.group_bys.len().hash(state);

        for value in &self.group_bys {
            match value {
                Value::Boolean(value) => {
                    0_u8.hash(state);
                    value.hash(state);
                }
                Value::SmallInt(value) => {
                    1_u8.hash(state);
                    value.hash(state);
                }
                Value::Integer(value) => {
                    2_u8.hash(state);
                    value.hash(state);
                }
                Value::BigInt(value) => {
                    3_u8.hash(state);
                    value.hash(state);
                }
                Value::Decimal(value) => {
                    4_u8.hash(state);
                    let value = if value.is_nan() {
                        f64::NAN
                    } else if *value == 0.0 {
                        0.0
                    } else {
                        *value
                    };
                    value.to_bits().hash(state);
                }
                Value::Varchar(value) => {
                    5_u8.hash(state);
                    value.hash(state);
                }
                Value::Null(sql_type) => {
                    6_u8.hash(state);
                    match sql_type {
                        SqlType::Boolean => 0_u8,
                        SqlType::SmallInt => 1_u8,
                        SqlType::Integer => 2_u8,
                        SqlType::BigInt => 3_u8,
                        SqlType::Decimal => 4_u8,
                        SqlType::Varchar => 5_u8,
                    }
                    .hash(state);
                }
            }
        }
    }
}

impl PartialEq for AggregateKey {
    fn eq(&self, other: &Self) -> bool {
        self.group_bys.len() == other.group_bys.len()
            && self
                .group_bys
                .iter()
                .zip(&other.group_bys)
                .all(|(left, right)| match (left, right) {
                    (Value::Boolean(left), Value::Boolean(right)) => left == right,
                    (Value::SmallInt(left), Value::SmallInt(right)) => left == right,
                    (Value::Integer(left), Value::Integer(right)) => left == right,
                    (Value::BigInt(left), Value::BigInt(right)) => left == right,
                    (Value::Decimal(left), Value::Decimal(right)) => {
                        (left.is_nan() && right.is_nan()) || left == right
                    }
                    (Value::Varchar(left), Value::Varchar(right)) => left == right,
                    (Value::Null(left), Value::Null(right)) => left == right,
                    _ => false,
                })
    }
}

impl Eq for AggregateKey {}

struct AggregateValue {
    values: Vec<Value>,
}

struct AggregationHashTable {
    aggregations: Vec<(AggregationType, PlannedExpression)>,
    table: HashMap<AggregateKey, AggregateValue>,
}

impl AggregationHashTable {
    fn new(aggregations: Vec<(AggregationType, PlannedExpression)>) -> Self {
        Self {
            aggregations,
            table: HashMap::new(),
        }
    }

    fn make_initial_value(&self) -> AggregateValue {
        let values = self
            .aggregations
            .iter()
            .map(|_| Value::Null(SqlType::Integer))
            .collect();
        AggregateValue { values }
    }

    fn combine_aggregate_values(
        aggregations: &[(AggregationType, PlannedExpression)],
        left: &mut AggregateValue,
        right: AggregateValue,
    ) -> Result<(), ExecutionError> {
        for (((aggregation_type, _), prev_val), new_val) in aggregations
            .iter()
            .zip(left.values.iter_mut())
            .zip(right.values)
        {
            match aggregation_type {
                AggregationType::Count => match (prev_val.is_null(), new_val.is_null()) {
                    (true, false) => *prev_val = Value::Integer(1),
                    (false, false) => {
                        *prev_val = eval_arithmetic(
                            &ArithmeticType::Plus,
                            prev_val.clone(),
                            Value::Integer(1),
                        )?;
                    }
                    _ => {}
                },
                AggregationType::Sum => match (prev_val.is_null(), new_val.is_null()) {
                    (true, false) => *prev_val = new_val,
                    (false, false) => {
                        *prev_val =
                            eval_arithmetic(&ArithmeticType::Plus, prev_val.clone(), new_val)?;
                    }
                    _ => {}
                },
                AggregationType::Min | AggregationType::Max => {
                    match (prev_val.is_null(), new_val.is_null()) {
                        (true, false) => *prev_val = new_val,
                        (false, false) => {
                            let comparison_type = match aggregation_type {
                                AggregationType::Min => ComparisonType::LessThan,
                                AggregationType::Max => ComparisonType::GreaterThan,
                                _ => unreachable!("only min and max use this branch"),
                            };

                            if eval_comparison_is_true(
                                new_val.clone(),
                                prev_val.clone(),
                                &comparison_type,
                            )? {
                                *prev_val = new_val;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(())
    }

    pub fn insert_combine(
        &mut self,
        key: AggregateKey,
        value: AggregateValue,
    ) -> Result<(), ExecutionError> {
        match self.table.entry(key) {
            Entry::Occupied(mut entry) => {
                Self::combine_aggregate_values(&self.aggregations, entry.get_mut(), value)
            }
            Entry::Vacant(entry) => {
                entry.insert(value);
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::planner::expression::{
        ConstantValueExpression, ExpressionType, PlannedExpressionKind,
    };

    fn aggregate(aggregation_type: AggregationType) -> AggregationHashTable {
        AggregationHashTable::new(vec![(
            aggregation_type,
            PlannedExpression {
                return_type: ExpressionType::new_integer(),
                kind: PlannedExpressionKind::ConstantValue(ConstantValueExpression {
                    value: Value::Integer(0),
                }),
            },
        )])
    }

    #[test]
    fn combines_min_and_max_while_ignoring_nulls() {
        let min = aggregate(AggregationType::Min);
        let mut min_value = AggregateValue {
            values: vec![Value::Null(SqlType::Integer)],
        };
        AggregationHashTable::combine_aggregate_values(
            &min.aggregations,
            &mut min_value,
            AggregateValue {
                values: vec![Value::Integer(3)],
            },
        )
        .unwrap();
        AggregationHashTable::combine_aggregate_values(
            &min.aggregations,
            &mut min_value,
            AggregateValue {
                values: vec![Value::Integer(1)],
            },
        )
        .unwrap();
        AggregationHashTable::combine_aggregate_values(
            &min.aggregations,
            &mut min_value,
            AggregateValue {
                values: vec![Value::Null(SqlType::Integer)],
            },
        )
        .unwrap();
        assert_eq!(min_value.values, vec![Value::Integer(1)]);

        let max = aggregate(AggregationType::Max);
        let mut max_value = AggregateValue {
            values: vec![Value::Varchar("ant".into())],
        };
        AggregationHashTable::combine_aggregate_values(
            &max.aggregations,
            &mut max_value,
            AggregateValue {
                values: vec![Value::Varchar("zebra".into())],
            },
        )
        .unwrap();
        assert_eq!(max_value.values, vec![Value::Varchar("zebra".into())]);
    }
}
