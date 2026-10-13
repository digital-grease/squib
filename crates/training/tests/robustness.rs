//! Plan 5.2: shared drill files, drill QR codes and manual time entry against malformed,
//! truncated, oversized and deeply nested input. Properties: no panic, bounded time, and
//! anything accepted is valid and round-trips. See testing/mutate.rs for run control.

#[path = "../../../testing/mutate.rs"]
mod mutate;

use std::io::Write;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use mutate::{JSON_TOKENS, deeply_nested, fuzz, iterations};
use squib_training::drill::{DrillRecipe, MAX_RECIPE_BYTES, starter_recipes};
use squib_training::manual::ManualString;
use squib_training::qr;

const LIMIT: Duration = Duration::from_millis(500);
const PREFIX: &str = "SQUIB-DRILL-1:";

fn pack(raw: &[u8]) -> String {
    let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(raw).unwrap();
    format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(z.finish().unwrap()))
}

fn recipe_seeds() -> Vec<Vec<u8>> {
    let mut v = vec![];
    for r in starter_recipes() {
        v.push(serde_json::to_vec(&r).unwrap());
        v.push(serde_json::to_vec_pretty(&r).unwrap());
    }
    v
}

/// What every accepted recipe must satisfy, however it arrived.
fn accepted(r: &DrillRecipe) -> Result<(), String> {
    r.validate().map_err(|e| format!("accepted but invalid: {e}"))?;
    let again = DrillRecipe::from_shared_bytes(&serde_json::to_vec(r).unwrap()).map_err(|e| format!("no round trip: {e}"))?;
    if &again != r || again.content_hash() != r.content_hash() {
        return Err("round trip changed the recipe".into());
    }
    match qr::encode(r) {
        Ok(text) if qr::decode(&text).as_ref() != Ok(r) => Err("QR round trip changed the recipe".into()),
        Ok(_) | Err(qr::QrError::TooLarge(_)) => Ok(()),
        Err(e) => Err(format!("accepted recipe cannot be encoded: {e}")),
    }
}

#[test]
fn robust_drill_files() {
    fuzz("drill file", &recipe_seeds(), JSON_TOKENS, iterations(1500), LIMIT, |b| match DrillRecipe::from_shared_bytes(b) {
        Ok(r) => accepted(&r),
        Err(_) => Ok(()),
    });
}

#[test]
fn robust_drill_qr_text() {
    let seeds: Vec<Vec<u8>> = starter_recipes().iter().map(|r| qr::encode(r).unwrap().into_bytes()).collect();
    let tokens: &[&[u8]] = &[b"A", b"_", b"-", b"=", b"+", b"/", b"SQUIB-DRILL-1:", b"SQUIB-DRILL-2:", b" ", b"\n", b"\xc3\xa9"];
    fuzz("drill qr text", &seeds, tokens, iterations(1500), LIMIT, |b| match qr::decode(&String::from_utf8_lossy(b)) {
        Ok(r) => accepted(&r),
        Err(_) => Ok(()),
    });
}

/// Mutate the recipe JSON, then pack it properly: the QR path must agree with the file path.
#[test]
fn robust_drill_qr_payloads_agree_with_files() {
    fuzz("drill qr payload", &recipe_seeds(), JSON_TOKENS, iterations(800), LIMIT, |b| {
        let via_qr = qr::decode(&pack(b));
        let via_file = DrillRecipe::from_shared_bytes(b);
        match (&via_qr, &via_file) {
            (Ok(a), Ok(f)) if a == f => accepted(a),
            (Err(_), Err(_)) => Ok(()),
            _ => Err(format!("QR and file disagree: {via_qr:?} vs {via_file:?}")),
        }
    });
}

/// Mutate the compressed stream itself: damaged deflate data is refused, never trusted.
#[test]
fn robust_drill_qr_deflate_streams() {
    let seeds: Vec<Vec<u8>> = recipe_seeds()
        .iter()
        .map(|j| {
            let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
            z.write_all(j).unwrap();
            z.finish().unwrap()
        })
        .collect();
    fuzz("drill qr deflate", &seeds, &[], iterations(1500), LIMIT, |b| {
        match qr::decode(&format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(b))) {
            Ok(r) => accepted(&r),
            Err(_) => Ok(()),
        }
    });
}

#[test]
fn robust_manual_times() {
    let seeds = vec![b"1.42, 1.83 2.25".to_vec(), b"0.9 1.2 1.5 1.8 2.1 2.4".to_vec(), b"12.345".to_vec()];
    let tokens: &[&[u8]] = &[
        b"1e300",
        b"-1e300",
        b"1e-300",
        b"inf",
        b"NaN",
        b"-0",
        b"3600",
        b"3600.001",
        b"9223372036.854775807",
        b"0x10",
        b"1_000",
        b",",
        b",,",
        b" ",
        b"\t",
        b"\xe2\x80\x83",
        b".",
        b"..",
        b"+",
        b"-",
        b"9999999999999999999999",
    ];
    for precision in [1_000_000, 10_000_000, 100_000_000, 0, -1, 7, i64::MAX, i64::MIN] {
        fuzz(&format!("manual times at {precision}"), &seeds, tokens, iterations(800), LIMIT, |b| {
            match ManualString::parse("Borrowed timer", precision, &String::from_utf8_lossy(b)) {
                Ok(m) => m.validate().map_err(|e| format!("accepted but invalid: {e}")),
                Err(_) => Ok(()),
            }
        });
    }
}

#[test]
fn deeply_nested_and_oversized_drills_are_refused() {
    // Under the size limit but nested past serde_json's recursion limit.
    let deep = deeply_nested(MAX_RECIPE_BYTES / 2 - 1);
    assert!(deep.len() <= MAX_RECIPE_BYTES);
    assert!(DrillRecipe::from_shared_bytes(&deep).is_err());
    assert!(qr::decode(&pack(&deep)).is_err());
    // Over the size limit, whatever the content.
    let mut big = serde_json::to_vec(&starter_recipes()[0]).unwrap();
    big.resize(MAX_RECIPE_BYTES + 1, b' ');
    assert!(DrillRecipe::from_shared_bytes(&big).is_err());
    // A compression bomb inflates only to the limit, then stops.
    let bomb = pack(&vec![b' '; 50 * 1024 * 1024]);
    let t = std::time::Instant::now();
    assert!(matches!(qr::decode(&bomb), Err(qr::QrError::Damaged)));
    assert!(t.elapsed() < Duration::from_secs(1), "bomb refused in {:?}", t.elapsed());
}

/// Inputs the fuzzer found, pinned so they stay fixed.
#[test]
fn regressions_found_by_fuzzing() {
    // A huge time overflowed the nanosecond conversion (panic in debug, wrap in release).
    for text in ["1e300", "-1e300", "9223372036.854775807", "1.2 1e300"] {
        assert!(ManualString::parse("Borrowed timer", 10_000_000, text).is_err(), "{text}");
    }
    assert!(ManualString::parse("Borrowed timer", i64::MIN, "1.5").is_err());
}
