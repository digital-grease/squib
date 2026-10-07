//! Squib's single authoritative SQLite repository (ADR-004).
//!
//! - Explicit numbered migrations tracked by `PRAGMA user_version`; each migration runs
//!   in one transaction and a pre-migration backup is written with the SQLite online
//!   backup API, so a failure keeps the original recoverable.
//! - Foreign keys on every connection, WAL journal, `synchronous=FULL` so an
//!   acknowledged commit is durable.
//! - Run intent is written before the run is armed. Observations are appended in
//!   numbered batches; `last_durable_seq` is the acknowledged high-water mark.
//! - Detected candidates, quality events, and revisions are append-only (triggers).
//! - Unfinished runs recover as interrupted with `uncommitted_tail_possible`.
//! - Raw PCM is never stored; only the coarse energy envelope.

mod records;
mod repo;

pub use records::*;
pub use repo::*;

use thiserror::Error;

pub const SCHEMA_VERSION: u32 = 1;

pub(crate) const MIGRATIONS: &[(u32, &str)] = &[(1, include_str!("migrations/0001_initial.sql"))];

/// Storage failures are reported distinctly (docs/squib/06).
#[derive(Debug, Error, Clone, PartialEq)]
pub enum StorageError {
    #[error("storage is full")]
    DiskFull,
    #[error("database integrity problem: {0}")]
    Integrity(String),
    #[error("migration to schema {version} failed: {message}; original database kept")]
    Migration { version: u32, message: String },
    #[error("database schema {found} is newer than this app supports ({supported})")]
    NewerSchema { found: u32, supported: u32 },
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid stored data: {0}")]
    Corrupt(String),
    #[error("write failed: {0}")]
    Write(String),
}

impl From<rusqlite::Error> for StorageError {
    fn from(e: rusqlite::Error) -> Self {
        use rusqlite::ErrorCode;
        match &e {
            rusqlite::Error::SqliteFailure(f, msg) => match f.code {
                ErrorCode::DiskFull => StorageError::DiskFull,
                ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase => {
                    StorageError::Integrity(msg.clone().unwrap_or_else(|| e.to_string()))
                }
                ErrorCode::ConstraintViolation => StorageError::Conflict(msg.clone().unwrap_or_else(|| e.to_string())),
                _ => StorageError::Write(e.to_string()),
            },
            rusqlite::Error::QueryReturnedNoRows => StorageError::NotFound(e.to_string()),
            _ => StorageError::Write(e.to_string()),
        }
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(e: serde_json::Error) -> Self {
        StorageError::Corrupt(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, StorageError>;
