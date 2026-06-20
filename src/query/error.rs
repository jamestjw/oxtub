use thiserror::Error;

#[derive(Debug, Error)]
pub enum QueryError {
    #[error("parse error: {0}")]
    Parse(#[from] sqlparser::parser::ParserError),

    #[error("expected a single statement")]
    ExpectedSingleStatement,

    #[error("unsupported statement: {0}")]
    UnsupportedStatement(&'static str),

    #[error("unsupported query: {0}")]
    UnsupportedQuery(&'static str),

    #[error("unsupported expression")]
    UnsupportedExpression,

    #[error("unsupported data type: {0}")]
    UnsupportedDataType(String),

    #[error("VARCHAR requires a size, e.g. VARCHAR(32)")]
    VarcharMissingSize,
}
