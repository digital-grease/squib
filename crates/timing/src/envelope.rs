//! Coarse, non-playable energy envelope (docs/squib/08 retention rules).
//!
//! One (RMS, peak) pair per 10 ms hop, quantized to 0.5 dB steps over −120..0 dBFS and
//! stored as two bytes. At 100 values/s this cannot reconstruct speech or PCM; it exists
//! only for diagnostics labelled "Energy", never "waveform".

use serde::{Deserialize, Serialize};

pub const ENVELOPE_VERSION: &str = "energy-10ms-q05db-v1";
pub const HOPS_PER_SECOND: u32 = 100;

pub fn quantize_db(db: f64) -> u8 {
    let q = ((db.clamp(-120.0, 0.0) + 120.0) * 2.0).round();
    q as u8
}

pub fn dequantize_db(q: u8) -> f32 {
    f32::from(q) / 2.0 - 120.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvelopeChunk {
    /// Frame index of the first hop.
    pub first_frame: i64,
    pub hop_frames: u32,
    /// Interleaved (rms_q, peak_q) bytes.
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct EnvelopeBuilder {
    hop: u32,
    acc_sq: f64,
    acc_peak: f32,
    acc_n: u32,
    chunk_first: Option<i64>,
    next_hop_frame: i64,
    data: Vec<u8>,
    hops_per_chunk: usize,
}

impl EnvelopeBuilder {
    pub fn new(sample_rate_hz: u32) -> Self {
        let hops_per_chunk = HOPS_PER_SECOND as usize;
        Self {
            hop: (sample_rate_hz / HOPS_PER_SECOND).max(1),
            acc_sq: 0.0,
            acc_peak: 0.0,
            acc_n: 0,
            chunk_first: None,
            next_hop_frame: 0,
            data: Vec::with_capacity(hops_per_chunk * 2),
            hops_per_chunk,
        }
    }

    pub fn push(&mut self, first_frame: i64, samples: &[f32], out: &mut impl FnMut(EnvelopeChunk)) {
        if self.chunk_first.is_none() && self.acc_n == 0 && self.data.is_empty() {
            self.next_hop_frame = first_frame;
        }
        for &x in samples {
            if self.acc_n == 0 && self.data.is_empty() {
                self.chunk_first = Some(self.next_hop_frame);
            }
            self.acc_sq += f64::from(x) * f64::from(x);
            self.acc_peak = self.acc_peak.max(x.abs());
            self.acc_n += 1;
            if self.acc_n == self.hop {
                let rms = (self.acc_sq / f64::from(self.hop)).sqrt();
                self.data.push(quantize_db(20.0 * rms.max(1e-9).log10()));
                self.data.push(quantize_db(20.0 * f64::from(self.acc_peak).max(1e-9).log10()));
                self.next_hop_frame += i64::from(self.hop);
                self.acc_sq = 0.0;
                self.acc_peak = 0.0;
                self.acc_n = 0;
                if self.data.len() >= self.hops_per_chunk * 2 {
                    self.flush(out);
                }
            }
        }
    }

    /// Emit any complete hops not yet emitted.
    pub fn flush(&mut self, out: &mut impl FnMut(EnvelopeChunk)) {
        if let Some(first) = self.chunk_first.take()
            && !self.data.is_empty()
        {
            let data = std::mem::replace(&mut self.data, Vec::with_capacity(self.hops_per_chunk * 2));
            out(EnvelopeChunk { first_frame: first, hop_frames: self.hop, data });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coarse_rate_and_quantization() {
        let mut b = EnvelopeBuilder::new(48_000);
        let mut chunks = vec![];
        let sig: Vec<f32> = (0..96_000).map(|i| if i < 48_000 { 0.5 } else { 0.05 }).collect();
        b.push(0, &sig, &mut |c| chunks.push(c));
        b.flush(&mut |c| chunks.push(c));
        let total: usize = chunks.iter().map(|c| c.data.len() / 2).sum();
        assert_eq!(total, 200, "100 hops per second");
        assert_eq!(chunks[0].first_frame, 0);
        assert_eq!(chunks[1].first_frame, 48_000);
        assert!((dequantize_db(chunks[0].data[0]) - (-6.0)).abs() <= 0.5);
        assert!((dequantize_db(chunks[1].data[0]) - (-26.0)).abs() <= 0.5);
        // 2 bytes per 480 input samples: far below any reconstructable representation.
        assert!(chunks.iter().map(|c| c.data.len()).sum::<usize>() * 480 <= sig.len() * 2);
    }
}
