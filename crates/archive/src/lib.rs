//! Squib portability (docs/squib/06 "Backup/export modes", "Import contract").
//!
//! - **Private backup**: full-fidelity, explicitly private, unencrypted zip with a
//!   manifest (format/version, schema version, file inventory with SHA-256, what kinds
//!   of private data are included) and one JSON file per table. Integers are decimal
//!   strings so 64-bit timestamps survive JSON consumers.
//! - **Import**: validated before any mutation (size, entry count, compression ratio,
//!   traversal/symlinks, unlisted files, hashes, schema version, identifiers), staged in
//!   a temporary database migrated with the app's own migrations, checked for
//!   integrity, then merged in one transaction. Identical records are skipped;
//!   divergent records are conflicts and abort the import. Hashes detect damage; they
//!   do not authenticate a hostile archive.
//! - **CSV**: analysis export with unit columns and spreadsheet-formula escaping.
//! - **Share results**: redacted subset with no location, station ids, notes,
//!   attachments, or device identifiers.

pub mod backup;
pub mod cell;
pub mod csv_export;
pub mod share;

pub use backup::*;

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum ArchiveError {
    #[error("archive is larger than the allowed limit")]
    TooLarge,
    #[error("archive has too many entries")]
    TooManyEntries,
    #[error("entry '{0}' is not allowed (path, type, or size)")]
    BadEntry(String),
    #[error("entry '{0}' expands suspiciously (possible zip bomb)")]
    CompressionRatio(String),
    #[error("not a Squib backup: {0}")]
    NotSquib(String),
    #[error("backup schema {found} is newer than this app supports ({supported}); update the app first")]
    NewerSchema { found: u32, supported: u32 },
    #[error("file '{0}' is damaged (hash mismatch)")]
    HashMismatch(String),
    #[error("file '{0}' is missing or not listed in the manifest")]
    Inventory(String),
    #[error("invalid data in '{table}': {detail}")]
    InvalidData { table: String, detail: String },
    #[error("{0} records conflict with different existing records; nothing was imported")]
    Conflicts(usize),
    #[error("storage: {0}")]
    Storage(String),
    #[error("io: {0}")]
    Io(String),
}

impl From<squib_storage::StorageError> for ArchiveError {
    fn from(e: squib_storage::StorageError) -> Self {
        ArchiveError::Storage(e.to_string())
    }
}

impl From<rusqlite::Error> for ArchiveError {
    fn from(e: rusqlite::Error) -> Self {
        ArchiveError::Storage(e.to_string())
    }
}

impl From<std::io::Error> for ArchiveError {
    fn from(e: std::io::Error) -> Self {
        ArchiveError::Io(e.to_string())
    }
}

impl From<zip::result::ZipError> for ArchiveError {
    fn from(e: zip::result::ZipError) -> Self {
        ArchiveError::NotSquib(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, ArchiveError>;

/// Format a UTC millisecond timestamp as ISO 8601 (`YYYY-MM-DDTHH:MM:SSZ`).
pub fn iso_utc(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    #[test]
    fn iso_dates() {
        assert_eq!(super::iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(super::iso_utc(1_791_411_300_000), "2026-10-07T22:15:00Z");
        assert_eq!(super::iso_utc(1_709_208_000_000), "2024-02-29T12:00:00Z");
    }
}
