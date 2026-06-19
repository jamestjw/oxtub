use crate::{
    buffer::bpm::BufferPoolManager,
    catalog::{schema::Schema, types::SqlType},
    storage::index::{
        b_tree_index::BTreeIndex,
        error::IndexError,
        index::{Index, IndexMetadata},
        key_encoder::{encode_one_col_key, encode_two_col_key, encoded_key_size},
    },
};

pub fn build_index<'a>(
    bpm: &'a BufferPoolManager,
    table_schema: &Schema,
    key_schema: &Schema,
    key_attrs: &[usize],
    key_size: usize,
    metadata: IndexMetadata,
) -> Result<Box<dyn Index + 'a>, IndexError> {
    validate_key_schema(table_schema, key_schema, key_attrs)?;

    let sql_types = key_schema
        .columns()
        .iter()
        .map(|col| col.sql_type())
        .collect::<Vec<_>>();

    let index: Box<dyn Index + 'a> = match (key_size, sql_types.as_slice()) {
        (_, [_]) => build_one_col_index(bpm, key_size, &sql_types, metadata)?,
        (_, [_, _]) => build_two_col_index(bpm, key_size, &sql_types, metadata)?,
        _ => return Err(IndexError::UnsupportedIndexType),
    };

    Ok(index)
}

fn build_one_col_index<'a>(
    bpm: &'a BufferPoolManager,
    key_size: usize,
    sql_types: &[SqlType],
    metadata: IndexMetadata,
) -> Result<Box<dyn Index + 'a>, IndexError> {
    if encoded_key_size(sql_types) != Some(key_size) {
        return Err(IndexError::UnsupportedIndexType);
    }

    let index: Box<dyn Index + 'a> = match key_size {
        1 => Box::new(BTreeIndex::<1>::new(bpm, metadata, encode_one_col_key::<1>)),
        2 => Box::new(BTreeIndex::<2>::new(bpm, metadata, encode_one_col_key::<2>)),
        4 => Box::new(BTreeIndex::<4>::new(bpm, metadata, encode_one_col_key::<4>)),
        8 => Box::new(BTreeIndex::<8>::new(bpm, metadata, encode_one_col_key::<8>)),
        _ => return Err(IndexError::UnsupportedIndexType),
    };

    Ok(index)
}

fn build_two_col_index<'a>(
    bpm: &'a BufferPoolManager,
    key_size: usize,
    sql_types: &[SqlType],
    metadata: IndexMetadata,
) -> Result<Box<dyn Index + 'a>, IndexError> {
    if encoded_key_size(sql_types) != Some(key_size) {
        return Err(IndexError::UnsupportedIndexType);
    }

    let index: Box<dyn Index + 'a> = match key_size {
        2 => Box::new(BTreeIndex::<2>::new(bpm, metadata, encode_two_col_key::<2>)),
        3 => Box::new(BTreeIndex::<3>::new(bpm, metadata, encode_two_col_key::<3>)),
        4 => Box::new(BTreeIndex::<4>::new(bpm, metadata, encode_two_col_key::<4>)),
        5 => Box::new(BTreeIndex::<5>::new(bpm, metadata, encode_two_col_key::<5>)),
        6 => Box::new(BTreeIndex::<6>::new(bpm, metadata, encode_two_col_key::<6>)),
        8 => Box::new(BTreeIndex::<8>::new(bpm, metadata, encode_two_col_key::<8>)),
        9 => Box::new(BTreeIndex::<9>::new(bpm, metadata, encode_two_col_key::<9>)),
        10 => Box::new(BTreeIndex::<10>::new(
            bpm,
            metadata,
            encode_two_col_key::<10>,
        )),
        12 => Box::new(BTreeIndex::<12>::new(
            bpm,
            metadata,
            encode_two_col_key::<12>,
        )),
        16 => Box::new(BTreeIndex::<16>::new(
            bpm,
            metadata,
            encode_two_col_key::<16>,
        )),
        _ => return Err(IndexError::UnsupportedIndexType),
    };

    Ok(index)
}

fn validate_key_schema(
    table_schema: &Schema,
    key_schema: &Schema,
    key_attrs: &[usize],
) -> Result<(), IndexError> {
    if key_schema.num_columns() != key_attrs.len() {
        return Err(IndexError::UnsupportedIndexType);
    }

    for (key_attr, key_col) in key_attrs.iter().zip(key_schema.columns()) {
        let Some(table_col) = table_schema.columns().get(*key_attr) else {
            return Err(IndexError::UnsupportedIndexType);
        };

        if table_col.sql_type() != key_col.sql_type() {
            return Err(IndexError::UnsupportedIndexType);
        }
    }

    Ok(())
}
