//! Drill sharing by QR code (plan phase 4.1, ADR-052; docs/squib/07: "QR codes later carry a
//! bounded payload ..., never executable instructions"; docs/squib/08: bounded decompression,
//! QR metadata cannot load code or paths).
//!
//! Payload: `SQUIB-DRILL-1:` followed by base64url (no padding) of the deflate-compressed
//! compact recipe JSON. It is data only: there is no link, nothing is fetched, and scanning
//! shows a preview before anything is saved. Decoding inflates with a hard output limit and
//! then applies the same validation as a shared file.

use std::io::Read;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::drill::{DrillError, DrillRecipe, MAX_RECIPE_BYTES};

pub const PREFIX: &str = "SQUIB-DRILL-1:";
/// Longest payload Squib puts in a QR code: fits a version 40 symbol at medium error
/// correction (2331 bytes in byte mode) with room to spare.
pub const MAX_QR_CHARS: usize = 2_000;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum QrError {
    #[error("this drill is too long for a QR code ({0} characters; the limit is {MAX_QR_CHARS}); share it as a file")]
    TooLarge(usize),
    #[error("not a Squib drill code")]
    NotADrill,
    #[error("the drill code is damaged or from a newer Squib")]
    Damaged,
    #[error(transparent)]
    Recipe(#[from] DrillError),
}

/// Encode a validated recipe as QR text, or say it is too large for one.
pub fn encode(recipe: &DrillRecipe) -> Result<String, QrError> {
    recipe.validate()?;
    let json = serde_json::to_vec(recipe).map_err(|_| QrError::Damaged)?;
    let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    std::io::Write::write_all(&mut z, &json).map_err(|_| QrError::Damaged)?;
    let packed = z.finish().map_err(|_| QrError::Damaged)?;
    let text = format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(packed));
    if text.len() > MAX_QR_CHARS {
        return Err(QrError::TooLarge(text.len()));
    }
    Ok(text)
}

/// Decode QR text into a validated recipe. Never fetches, never runs anything, and never
/// inflates past the shared-file size limit.
pub fn decode(text: &str) -> Result<DrillRecipe, QrError> {
    let body = text.trim().strip_prefix(PREFIX).ok_or(QrError::NotADrill)?;
    if body.len() > MAX_QR_CHARS * 2 {
        return Err(QrError::Damaged);
    }
    let packed = URL_SAFE_NO_PAD.decode(body).map_err(|_| QrError::Damaged)?;
    let mut json = Vec::new();
    // One byte over the limit tells "too large" apart from "exactly at the limit".
    flate2::read::DeflateDecoder::new(packed.as_slice())
        .take(MAX_RECIPE_BYTES as u64 + 1)
        .read_to_end(&mut json)
        .map_err(|_| QrError::Damaged)?;
    if json.len() > MAX_RECIPE_BYTES {
        return Err(QrError::Damaged);
    }
    Ok(DrillRecipe::from_shared_bytes(&json)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drill::starter_recipes;

    fn pack(raw: &[u8]) -> String {
        let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
        std::io::Write::write_all(&mut z, raw).unwrap();
        format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(z.finish().unwrap()))
    }

    #[test]
    fn starter_drills_round_trip_well_inside_the_limit() {
        for r in starter_recipes() {
            let t = encode(&r).unwrap();
            assert!(t.starts_with(PREFIX) && t.len() < 400, "{} chars", t.len());
            assert_eq!(decode(&t).unwrap(), r);
            assert_eq!(decode(&format!("  {t}\n")).unwrap(), r, "surrounding whitespace from scanners");
        }
    }

    #[test]
    fn long_drills_are_refused_with_a_file_hint() {
        let mut r = starter_recipes().remove(0);
        // Text that compresses poorly: pseudo-random letters (a fixed linear congruential sequence).
        let mut x: u64 = 12_345;
        let mut letters = |n: usize| -> String {
            (0..n)
                .map(|_| {
                    x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
                    char::from(b'a' + ((x >> 33) % 26) as u8)
                })
                .collect()
        };
        r.description = letters(999);
        r.notes = letters(1999);
        assert!(matches!(encode(&r), Err(QrError::TooLarge(_))));
        assert!(encode(&r).unwrap_err().to_string().contains("share it as a file"));
    }

    #[test]
    fn hostile_or_foreign_codes_are_rejected_without_work() {
        assert_eq!(decode("https://example.com/drill.json"), Err(QrError::NotADrill));
        assert_eq!(decode("SQUIB-DRILL-2:abc"), Err(QrError::NotADrill));
        assert_eq!(decode(&format!("{PREFIX}not base64!")), Err(QrError::Damaged));
        // A decompression bomb: 1 MB of spaces packs into about 1.4 KB but is cut at 16 KB.
        let bomb = pack(&vec![b' '; 1024 * 1024]);
        assert!(bomb.len() < MAX_QR_CHARS * 2, "{}", bomb.len());
        assert_eq!(decode(&bomb), Err(QrError::Damaged));
        // Valid compression, invalid recipe: the shared-file validation applies.
        assert!(matches!(decode(&pack(br#"{"title":""}"#)), Err(QrError::Recipe(_))));
        let mut bad = starter_recipes().remove(0);
        bad.repeats = 999;
        let json = serde_json::to_vec(&bad).unwrap();
        assert!(matches!(decode(&pack(&json)), Err(QrError::Recipe(_))));
        // Oversized text (a larger bomb, for example) is refused before decoding.
        assert_eq!(decode(&pack(&vec![b' '; 10 * 1024 * 1024])), Err(QrError::Damaged));
        assert_eq!(decode(&format!("{PREFIX}{}", "A".repeat(MAX_QR_CHARS * 2 + 1))), Err(QrError::Damaged));
    }
}
