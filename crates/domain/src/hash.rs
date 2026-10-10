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

/// SHA-256 and length of a file, read in chunks so large clips are not held in memory.
pub fn sha256_file(path: &std::path::Path) -> std::io::Result<(String, u64)> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut n = 0u64;
    loop {
        let r = f.read(&mut buf)?;
        if r == 0 {
            break;
        }
        h.update(&buf[..r]);
        n += r as u64;
    }
    Ok((hex(&h.finalize()), n))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}
