//! Drill sharing by QR code (plan phase 4.1, ADR-052). Encoding and decoding both happen
//! here, in the core: the platform only draws the module grid and hands over camera frames.

use qrcodegen::{QrCode, QrCodeEcc};
use squib_training::drill::DrillVersion;
use squib_training::qr;

use crate::engine::SquibEngine;
use crate::ffi::SquibError;
use crate::training_api::{DrillView, drill_view};

fn inval(e: impl ToString) -> SquibError {
    SquibError::Invalid(e.to_string())
}

/// A QR symbol as a square grid of modules, row by row (1 = dark), plus the text it holds.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct QrCodeView {
    pub size: u32,
    pub modules: Vec<u8>,
    pub text: String,
}

/// Find a QR code in a greyscale (luma) camera frame. Returns the text of the first
/// readable code, whatever it says; [`SquibEngine::preview_drill_qr`] decides if it is a drill.
#[uniffi::export]
pub fn decode_qr_luma(width: u32, height: u32, row_stride: u32, luma: Vec<u8>) -> Option<String> {
    let (w, h, stride) = (width as usize, height as usize, row_stride as usize);
    if w == 0 || h == 0 || stride < w || luma.len() < stride * (h - 1) + w || w * h > 4_000_000 {
        return None;
    }
    let mut img = rqrr::PreparedImage::prepare_from_greyscale(w, h, |x, y| luma[y * stride + x]);
    img.detect_grids().into_iter().find_map(|g| g.decode().ok().map(|(_, text)| text))
}

#[uniffi::export]
impl SquibEngine {
    /// QR code for one drill version, or a message saying to share it as a file.
    pub fn drill_qr(&self, drill_id: String, version: u32) -> Result<QrCodeView, SquibError> {
        let v = self.read_repo().load_drill_version(&drill_id, version)?.ok_or_else(|| SquibError::NotFound(drill_id.clone()))?;
        let text = qr::encode(&v.recipe).map_err(inval)?;
        let code = QrCode::encode_text(&text, QrCodeEcc::Medium).map_err(|_| inval("drill is too long for a QR code"))?;
        let size = code.size();
        let modules =
            (0..size).flat_map(|y| (0..size).map(move |x| (x, y))).map(|(x, y)| u8::from(code.get_module(x, y))).collect();
        Ok(QrCodeView { size: size as u32, modules, text })
    }

    /// What a scanned code would import, without saving anything.
    pub fn preview_drill_qr(&self, text: String, now_utc_ms: i64) -> Result<DrillView, SquibError> {
        let r = qr::decode(&text).map_err(inval)?;
        let v = DrillVersion::first("preview", r, now_utc_ms).map_err(inval)?;
        Ok(drill_view(&v))
    }

    /// Save a scanned drill as a new drill (validated data, never executed).
    pub fn import_drill_qr(&self, text: String, now_utc_ms: i64) -> Result<DrillView, SquibError> {
        let r = qr::decode(&text).map_err(inval)?;
        let v = DrillVersion::first(&uuid::Uuid::new_v4().to_string(), r, now_utc_ms).map_err(inval)?;
        let view = drill_view(&v);
        self.store_actor().exec(move |repo| repo.save_drill_version(&v))?;
        Ok(view)
    }
}
