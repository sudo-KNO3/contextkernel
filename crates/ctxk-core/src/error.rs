use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid value for {field}: {value}")]
    Invalid { field: String, value: String },

    #[error("parse error: {0}")]
    Parse(String),

    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}
