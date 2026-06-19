use crate::{
    catalog::schema::Schema,
    storage::{index::generic_key::GenericKey, table::tuple::Tuple},
    types::value::Value,
};

pub fn encode_i32_key(tuple: &Tuple, schema: &Schema) -> GenericKey<4> {
    match tuple.get_value(schema, 0) {
        Value::Integer(v) => GenericKey::<4>::from_i32(v),
        _ => panic!("invalid type"),
    }
}
