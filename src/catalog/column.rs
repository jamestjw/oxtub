use crate::{catalog::types::SqlType, storage::table::tuple::VarOffset};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    name: String,
    sql_type: SqlType,
    // offset within the tuple for data corresponding to the column
    pub(crate) value_offset: usize,
    size: ColumnSize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ColumnSize {
    Inline(usize),
    Variable(usize),
}

impl Column {
    pub fn new_static(name: String, sql_type: SqlType) -> Self {
        assert_ne!(sql_type, SqlType::Varchar);
        Self {
            name,
            sql_type,
            value_offset: 0,
            size: ColumnSize::Inline(sql_type.inline_size()),
        }
    }

    pub fn new_variable(name: String, sql_type: SqlType, size: usize) -> Self {
        assert_eq!(sql_type, SqlType::Varchar);
        Self {
            name,
            sql_type,
            value_offset: 0,
            size: ColumnSize::Variable(size),
        }
    }

    pub fn with_new_name(&self, name: String) -> Self {
        Self {
            name,
            ..self.clone()
        }
    }

    pub fn sql_type(&self) -> SqlType {
        self.sql_type
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    // size required to store the inlined part of this column
    pub fn inline_size(&self) -> usize {
        match self.size {
            ColumnSize::Inline(s) => s,
            ColumnSize::Variable(_) => size_of::<VarOffset>(),
        }
    }

    pub fn declared_size(&self) -> usize {
        match self.size {
            ColumnSize::Inline(s) => s,
            ColumnSize::Variable(s) => s,
        }
    }

    pub fn is_inlined(&self) -> bool {
        self.sql_type.is_inlined()
    }
}
