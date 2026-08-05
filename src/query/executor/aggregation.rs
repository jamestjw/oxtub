use std::collections::HashMap;

use crate::{
    catalog::types::SqlType,
    query::{binder::expression::AggregationType, planner::expression::PlannedExpression},
    types::value::Value,
};

struct AggregateKey {
    group_bys: Vec<Value>,
}

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

    fn combine_aggregate_values(&self, left: &mut AggregateValue, right: AggregateValue) {
        for (aggregation_type, planned_expression) in &self.aggregations {
            match aggregation_type {
                AggregationType::Count => todo!(),
                AggregationType::Sum => todo!(),
                AggregationType::Min => todo!(),
                AggregationType::Max => todo!(),
            }
        }
    }
}
