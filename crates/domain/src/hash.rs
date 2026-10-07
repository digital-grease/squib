//! Content hashing for configuration snapshots and review revisions.

use serde::Serialize;
use sha2::{Digest, Sha256};

/// Hex SHA-256 of the canonical serde_json encoding of `value`.
///
/// serde_json serializes struct fields in declaration order, so a type's encoding is
/// stable for a given schema version. Maps must be `BTreeMap` to stay canonical.
pub fn content_hash<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).expect("domain values serialize");
    hex(&Sha256::digest(&bytes))
}

/// Hex SHA-256 of raw bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}
