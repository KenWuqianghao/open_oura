//! Error type for the storage layer.
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("storage error: {0}")]
    Storage(String),
    /// The database was written by a newer schema than this build understands.
    #[error("database schema {found} is newer than the supported schema {supported}; update the app")]
    SchemaTooNew { found: i64, supported: i64 },
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Storage(e.to_string())
    }
}
