//! Plan phase 4.1: a drill becomes a QR code, the code is read back from a camera-like
//! greyscale frame, previewed without saving, then imported as data.

use std::sync::Arc;

use squib_mobile::*;

const NOW: i64 = 1_791_694_800_000;

fn engine(name: &str) -> Arc<SquibEngine> {
    let d = std::env::temp_dir().join(format!("squib-qr-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    SquibEngine::new(d.join("journal.db").to_string_lossy().into(), NOW).unwrap()
}

/// Draw the code like a phone screen seen by a camera: `px` pixels per module, a 4-module
/// quiet zone, optional 90° rotation, mild noise, and a row stride wider than the image.
fn frame(q: &QrCodeView, px: usize, rotate: bool) -> (u32, u32, u32, Vec<u8>) {
    let n = q.size as usize;
    let side = (n + 8) * px;
    let stride = side + 16;
    let mut img = vec![0u8; stride * side];
    let mut seed: u32 = 7;
    for y in 0..side {
        for x in 0..side {
            let (mx, my) = (x / px, y / px);
            let (mx, my) = if rotate { (my, n + 7 - mx) } else { (mx, my) };
            let dark = (4..n + 4).contains(&mx) && (4..n + 4).contains(&my) && q.modules[(my - 4) * n + (mx - 4)] == 1;
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let noise = (seed >> 16) as u8 % 40;
            img[y * stride + x] = if dark { 20 + noise } else { 200 + noise / 2 };
        }
    }
    (side as u32, side as u32, stride as u32, img)
}

#[test]
fn a_drill_round_trips_through_a_qr_code_and_imports_only_after_preview() {
    let e = engine("roundtrip");
    let drill = e.list_drills().into_iter().find(|d| d.input.title == "Repeated par").unwrap();
    let q = e.drill_qr(drill.drill_id.clone(), drill.version).unwrap();
    assert!(q.text.starts_with("SQUIB-DRILL-1:"));
    assert_eq!(q.modules.len(), (q.size * q.size) as usize);
    for rotate in [false, true] {
        let (w, h, stride, luma) = frame(&q, 4, rotate);
        assert_eq!(decode_qr_luma(w, h, stride, luma).as_deref(), Some(q.text.as_str()), "rotated {rotate}");
    }
    // A blank frame and a bad buffer find nothing and do not panic.
    assert_eq!(decode_qr_luma(200, 200, 200, vec![255; 40_000]), None);
    assert_eq!(decode_qr_luma(200, 200, 200, vec![0; 10]), None);

    let before = e.list_drills().len();
    let p = e.preview_drill_qr(q.text.clone(), NOW).unwrap();
    assert_eq!(p.input.title, "Repeated par");
    assert_eq!(e.list_drills().len(), before, "preview saves nothing");
    let saved = e.import_drill_qr(q.text.clone(), NOW).unwrap();
    assert_ne!(saved.drill_id, drill.drill_id, "a new drill, not an overwrite");
    assert_eq!(e.list_drills().len(), before + 1);
    // Anything that is not a Squib drill code is refused.
    assert!(e.preview_drill_qr("https://example.com/x".into(), NOW).is_err());
}
