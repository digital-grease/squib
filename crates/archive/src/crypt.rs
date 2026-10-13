//! Optional passphrase encryption of a private backup (plan 5.7, owner ruling D6).
//!
//! An encrypted backup is the ordinary backup zip wrapped whole in a standard age file
//! (<https://age-encryption.org/v1>) with a single scrypt passphrase recipient, written
//! by the `age` crate. Nothing here is new cryptography: age provides the key derivation
//! (scrypt), the file-key wrapping, and the authenticated streaming payload. A Squib
//! encrypted backup can be decrypted by any age implementation (`age -d`), which yields
//! the same zip an unencrypted backup would have been.
//!
//! The zip is built, and on import read back, as a plaintext temporary file in the
//! app's private scratch directory; [`Plain`] removes it on every path, including
//! errors. The journal it came from sits in the same private storage unencrypted.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

pub use age::secrecy::{ExposeSecret, SecretString};

use crate::{ArchiveError, Result};

/// Every binary age v1 file starts with this line.
pub const AGE_MAGIC: &[u8] = b"age-encryption.org/v1\n";
/// The ASCII-armoured form, which Squib does not write; recognised to give a clear error.
const ARMOR_MAGIC: &[u8] = b"-----BEGIN AGE ENCRYPTED FILE-----";

/// scrypt work factor (N = 2^17, 128 MiB) for backups Squib writes. Fixed rather than
/// age's per-device calibration, so a backup made on a fast phone still opens on a slow
/// one, and so memory use stays bounded on phones.
pub const WORK_FACTOR: u8 = 17;
/// Highest work factor accepted when opening a backup (N = 2^18, 256 MiB): one step of
/// headroom for files made with other age tools, while a hostile header cannot make the
/// app allocate gigabytes.
pub const MAX_WORK_FACTOR: u8 = 18;
pub const MIN_PASSPHRASE_CHARS: usize = 8;
const MAX_PASSPHRASE_CHARS: usize = 1024;

/// Whether a passphrase is acceptable for a new encrypted backup.
pub fn check_passphrase(p: &str) -> Result<()> {
    let n = p.chars().count();
    if n < MIN_PASSPHRASE_CHARS || p.trim().chars().count() < MIN_PASSPHRASE_CHARS {
        return Err(ArchiveError::Passphrase(format!("use at least {MIN_PASSPHRASE_CHARS} characters")));
    }
    if n > MAX_PASSPHRASE_CHARS {
        return Err(ArchiveError::Passphrase(format!("use at most {MAX_PASSPHRASE_CHARS} characters")));
    }
    if p.chars().any(char::is_control) {
        return Err(ArchiveError::Passphrase("remove control characters".into()));
    }
    Ok(())
}

/// What kind of file a picked backup is, from its first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encryption {
    None,
    Age,
}

pub fn detect(path: &Path) -> Result<Encryption> {
    let mut head = [0u8; 34];
    let mut f = File::open(path)?;
    let mut n = 0;
    while n < head.len() {
        match f.read(&mut head[n..])? {
            0 => break,
            r => n += r,
        }
    }
    let head = &head[..n];
    if head.starts_with(AGE_MAGIC) {
        Ok(Encryption::Age)
    } else if head.starts_with(ARMOR_MAGIC) {
        Err(ArchiveError::NotSquib("an armoured age file; Squib reads the binary form only".into()))
    } else {
        Ok(Encryption::None)
    }
}

/// A plaintext archive on disk: either the caller's own file, or a temporary one this
/// module made and deletes when dropped.
#[derive(Debug)]
pub struct Plain {
    path: PathBuf,
    temporary: bool,
}

impl Plain {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A fresh temporary path in `scratch_dir`, deleted when the value drops.
    pub fn temporary(scratch_dir: &Path, tag: &str) -> Plain {
        let path = scratch_dir.join(format!("squib-plain-{}-{tag}.zip", std::process::id()));
        let _ = std::fs::remove_file(&path);
        Plain { path, temporary: true }
    }
}

impl Drop for Plain {
    fn drop(&mut self) {
        if self.temporary {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Encrypt `plain` to `out` with `passphrase` at the given scrypt work factor.
pub fn encrypt_file(plain: &Path, out: &Path, passphrase: &SecretString, work_factor: u8) -> Result<()> {
    let mut recipient = age::scrypt::Recipient::new(passphrase.clone());
    recipient.set_work_factor(work_factor.clamp(1, MAX_WORK_FACTOR));
    let enc = age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
        .map_err(|e| ArchiveError::Io(format!("encryption: {e}")))?;
    let result = (|| -> Result<()> {
        let mut w = enc.wrap_output(BufWriter::new(File::create(out)?))?;
        std::io::copy(&mut BufReader::new(File::open(plain)?), &mut w)?;
        w.finish()?.flush()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(out);
    }
    result
}

/// Write a private backup encrypted with `passphrase`: the zip is built in `scratch_dir`,
/// encrypted to `out`, and the plaintext removed. `bytes` in the summary is the size of
/// the encrypted file.
pub fn export_encrypted(
    repo: &mut squib_storage::Repository,
    out: &Path,
    scratch_dir: &Path,
    opts: &crate::ExportOptions,
    passphrase: &SecretString,
) -> Result<crate::ExportSummary> {
    check_passphrase(passphrase.expose_secret())?;
    let plain = Plain::temporary(scratch_dir, "out");
    let mut summary = crate::export_private(repo, plain.path(), opts)?;
    encrypt_file(plain.path(), out, passphrase, WORK_FACTOR)?;
    summary.bytes = std::fs::metadata(out)?.len();
    Ok(summary)
}

/// Open a picked backup for reading. An unencrypted file is used where it is; an age
/// file is decrypted with `passphrase` into a temporary file in `scratch_dir`, at most
/// `max_bytes` long. The temporary file is removed when the returned [`Plain`] drops,
/// and immediately on any failure.
pub fn open(archive: &Path, scratch_dir: &Path, passphrase: Option<&SecretString>, max_bytes: u64) -> Result<Plain> {
    match detect(archive)? {
        Encryption::None => Ok(Plain { path: archive.to_path_buf(), temporary: false }),
        Encryption::Age => {
            let passphrase = passphrase.ok_or(ArchiveError::PassphraseRequired)?;
            let plain = Plain::temporary(scratch_dir, "in");
            decrypt_to(archive, plain.path(), passphrase, max_bytes)?;
            Ok(plain)
        }
    }
}

fn decrypt_to(archive: &Path, out: &Path, passphrase: &SecretString, max_bytes: u64) -> Result<()> {
    let dec = age::Decryptor::new_buffered(BufReader::new(File::open(archive)?)).map_err(decrypt_error)?;
    if !dec.is_scrypt() {
        return Err(ArchiveError::NotSquib("encrypted to a key, not a passphrase".into()));
    }
    let mut identity = age::scrypt::Identity::new(passphrase.clone());
    identity.set_max_work_factor(MAX_WORK_FACTOR);
    let mut r = dec.decrypt(std::iter::once(&identity as &dyn age::Identity)).map_err(decrypt_error)?;
    let mut w = BufWriter::new(File::create(out)?);
    // Read at most one byte past the limit; the stream authenticates each chunk, so a
    // damaged or truncated file fails here with an error rather than yielding data.
    let copied = std::io::copy(&mut (&mut r).take(max_bytes + 1), &mut w).map_err(|e| match e.kind() {
        std::io::ErrorKind::InvalidData | std::io::ErrorKind::UnexpectedEof => ArchiveError::Damaged,
        _ => ArchiveError::Io(e.to_string()),
    })?;
    if copied > max_bytes {
        return Err(ArchiveError::TooLarge);
    }
    w.flush()?;
    Ok(())
}

fn decrypt_error(e: age::DecryptError) -> ArchiveError {
    use age::DecryptError as D;
    match e {
        D::DecryptionFailed | D::NoMatchingKeys | D::KeyDecryptionFailed => ArchiveError::WrongPassphrase,
        D::ExcessiveWork { required, .. } => ArchiveError::NotSquib(format!(
            "its passphrase protection needs more memory than a phone should use (work factor {required}, \
             at most {MAX_WORK_FACTOR} accepted)"
        )),
        D::InvalidMac => ArchiveError::Damaged,
        D::Io(e) if matches!(e.kind(), std::io::ErrorKind::UnexpectedEof) => ArchiveError::Damaged,
        D::Io(e) => ArchiveError::Io(e.to_string()),
        other => ArchiveError::NotSquib(format!("unreadable encrypted file ({other})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("squib-crypt-unit-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn secret(s: &str) -> SecretString {
        SecretString::from(s.to_string())
    }

    #[test]
    fn passphrase_rules() {
        assert!(check_passphrase("correct horse").is_ok());
        assert!(check_passphrase("ünïcødé!").is_ok(), "eight characters, more bytes");
        for bad in ["", "short", "       a", "tab\tinside long", &"x".repeat(1025)] {
            assert!(check_passphrase(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn round_trip_wrong_passphrase_and_cleanup() {
        let d = tmp("rt");
        let plain = d.join("p.zip");
        let body: Vec<u8> = (0..200_000u32).map(|i| (i * 7 % 251) as u8).collect();
        std::fs::write(&plain, &body).unwrap();
        let enc = d.join("p.zip.age");
        encrypt_file(&plain, &enc, &secret("correct horse"), 10).unwrap();
        assert_eq!(detect(&enc).unwrap(), Encryption::Age);
        assert_eq!(detect(&plain).unwrap(), Encryption::None);
        let raw = std::fs::read(&enc).unwrap();
        assert!(!raw.windows(64).any(|w| w == &body[..64]), "payload is not plaintext");

        let scratch = d.join("scratch");
        std::fs::create_dir_all(&scratch).unwrap();
        let p = open(&enc, &scratch, Some(&secret("correct horse")), 1 << 20).unwrap();
        assert_eq!(std::fs::read(p.path()).unwrap(), body);
        let temp = p.path().to_path_buf();
        drop(p);
        assert!(!temp.exists(), "temporary plaintext removed on drop");

        assert_eq!(open(&enc, &scratch, Some(&secret("wrong horse")), 1 << 20).unwrap_err(), ArchiveError::WrongPassphrase);
        assert_eq!(open(&enc, &scratch, None, 1 << 20).unwrap_err(), ArchiveError::PassphraseRequired);
        assert_eq!(open(&enc, &scratch, Some(&secret("correct horse")), 1000).unwrap_err(), ArchiveError::TooLarge);
        // A plain file opens in place, passphrase or not, and is never deleted.
        let q = open(&plain, &scratch, Some(&secret("ignored here")), 1 << 20).unwrap();
        assert_eq!(q.path(), plain);
        drop(q);
        assert!(plain.exists());
        assert_eq!(std::fs::read_dir(&scratch).unwrap().count(), 0, "no plaintext left behind after failures");
    }

    #[test]
    fn tampering_and_truncation_are_detected() {
        let d = tmp("tamper");
        let plain = d.join("p.zip");
        std::fs::write(&plain, vec![42u8; 150_000]).unwrap();
        let enc = d.join("p.age");
        encrypt_file(&plain, &enc, &secret("correct horse"), 10).unwrap();
        let good = std::fs::read(&enc).unwrap();
        let scratch = d.join("s");
        std::fs::create_dir_all(&scratch).unwrap();
        let bad = d.join("bad.age");
        // Flip a payload byte in the last chunk, then truncate a chunk away.
        let mut flipped = good.clone();
        let n = flipped.len();
        flipped[n - 5] ^= 1;
        std::fs::write(&bad, &flipped).unwrap();
        assert_eq!(open(&bad, &scratch, Some(&secret("correct horse")), 1 << 20).unwrap_err(), ArchiveError::Damaged);
        std::fs::write(&bad, &good[..good.len() - 70_000]).unwrap();
        assert_eq!(open(&bad, &scratch, Some(&secret("correct horse")), 1 << 20).unwrap_err(), ArchiveError::Damaged);
        assert_eq!(std::fs::read_dir(&scratch).unwrap().count(), 0);
    }

    #[test]
    fn hostile_work_factor_is_refused_without_running_scrypt() {
        let d = tmp("work");
        let plain = d.join("p.zip");
        std::fs::write(&plain, b"x").unwrap();
        let enc = d.join("p.age");
        encrypt_file(&plain, &enc, &secret("correct horse"), 10).unwrap();
        // The stanza line is "-> scrypt <salt> 10"; claim 2^40 instead.
        let text = std::fs::read(&enc).unwrap();
        let pos = text.windows(3).position(|w| w == b" 10").unwrap();
        let mut hostile = text[..pos].to_vec();
        hostile.extend_from_slice(b" 40");
        hostile.extend_from_slice(&text[pos + 3..]);
        std::fs::write(&enc, &hostile).unwrap();
        let start = std::time::Instant::now();
        let e = open(&enc, &d, Some(&secret("correct horse")), 1 << 20).unwrap_err();
        assert!(matches!(e, ArchiveError::NotSquib(ref m) if m.contains("work factor 40")), "{e:?}");
        assert!(start.elapsed().as_secs() < 5);
    }
}
