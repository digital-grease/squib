//! Plan 5.2 at the binding surface: camera frames handed to the QR reader, and hostile
//! drill files and backups handed to the engine. Properties: no panic, bounded time, and
//! the engine keeps working afterwards (no poisoned lock). See testing/mutate.rs.

#[path = "../../../testing/mutate.rs"]
mod mutate;

use std::sync::Arc;
use std::time::Duration;

use mutate::{JSON_TOKENS, Rng, fuzz, iterations};
use squib_mobile::*;

const NOW: i64 = 1_791_694_800_000;

fn engine(name: &str) -> (Arc<SquibEngine>, std::path::PathBuf) {
    let d = std::env::temp_dir().join(format!("squib-robust-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    (SquibEngine::new(d.join("journal.db").to_string_lossy().into(), NOW).unwrap(), d)
}

/// A code drawn at `px` pixels per module with a quiet zone, as in qr_flows.rs.
fn frame(q: &QrCodeView, px: usize) -> (u32, u32, Vec<u8>) {
    let n = q.size as usize;
    let side = (n + 8) * px;
    let mut img = vec![220u8; side * side];
    for y in 0..side {
        for x in 0..side {
            let (mx, my) = (x / px, y / px);
            if (4..n + 4).contains(&mx) && (4..n + 4).contains(&my) && q.modules[(my - 4) * n + (mx - 4)] == 1 {
                img[y * side + x] = 30;
            }
        }
    }
    (side as u32, side as u32, img)
}

#[test]
fn robust_qr_camera_frames() {
    let (e, _) = engine("qr");
    let mut seeds = vec![];
    let mut dims = vec![];
    for d in e.list_drills().iter().take(3) {
        let q = e.drill_qr(d.drill_id.clone(), d.version).unwrap();
        let (w, h, img) = frame(&q, 3);
        assert!(decode_qr_luma(w, h, w, img.clone()).is_some(), "seed frame reads");
        dims.push((w, h));
        seeds.push(img);
    }
    // Every seed is the same size here (same QR version), so mutants keep a valid geometry
    // unless a mutation changes the buffer length, which the reader must also survive.
    let (w, h) = dims[0];
    assert!(dims.iter().all(|d| *d == (w, h)));
    let mut read = 0;
    fuzz("qr frame", &seeds, &[], iterations(150), Duration::from_secs(3), |img| {
        if let Some(text) = decode_qr_luma(w, h, w, img.to_vec()) {
            read += 1;
            let _ = e.preview_drill_qr(text, NOW);
        }
        Ok(())
    });
    eprintln!("qr frames still readable after mutation: {read}");
    // Arbitrary geometry: small random sizes, strides and buffers.
    let mut rng = Rng::new(mutate::base_seed() ^ 0x9e0);
    fuzz("qr geometry", &[vec![0]], &[], iterations(300), Duration::from_secs(2), |_| {
        let (w, h) = (rng.below(300) as u32, rng.below(300) as u32);
        let stride = w + rng.below(8) as u32;
        let len = rng.below((stride * h + 16) as usize + 1);
        let fill = rng.next() as u8;
        let mut buf = vec![fill; len];
        for _ in 0..rng.below(64) {
            if !buf.is_empty() {
                let i = rng.below(buf.len());
                buf[i] = rng.next() as u8;
            }
        }
        let _ = decode_qr_luma(w, h, stride, buf);
        Ok(())
    });
    assert_eq!(decode_qr_luma(u32::MAX, u32::MAX, u32::MAX, vec![0; 16]), None);
    assert_eq!(decode_qr_luma(4000, 4000, 4000, vec![0; 16]), None, "over the pixel cap");
}

#[test]
fn robust_engine_survives_hostile_drill_files_and_backups() {
    let (e, d) = engine("engine");
    let before = e.list_drills().len();
    let seeds: Vec<Vec<u8>> =
        e.list_drills().iter().take(3).map(|v| e.drill_file(v.drill_id.clone(), v.version).unwrap()).collect();
    let mut imported = 0;
    fuzz("engine drill file", &seeds, JSON_TOKENS, iterations(300), Duration::from_secs(1), |b| {
        if e.import_drill_file(b.to_vec(), NOW).is_ok() {
            imported += 1;
        }
        Ok(())
    });
    assert_eq!(e.list_drills().len(), before + imported, "every accepted file is one new drill, nothing else");

    let bad = d.join("bad.zip");
    for b in [b"".to_vec(), b"PK\x03\x04".to_vec(), vec![0xff; 4096], mutate::deeply_nested(100_000)] {
        std::fs::write(&bad, &b).unwrap();
        let p = bad.to_string_lossy().to_string();
        let s = d.to_string_lossy().to_string();
        assert!(e.import_backup(p.clone(), s.clone(), None).is_err());
        assert!(e.import_backup_copy(p, s, None, NOW).is_err());
    }
    // Still fully usable: reads and a write both succeed.
    assert_eq!(e.list_drills().len(), before + imported);
    let mut dv = e.list_drills().remove(0).input;
    dv.title = "After the fuzz".into();
    assert!(e.create_drill(dv, NOW).is_ok());
}
