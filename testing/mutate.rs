//! Dependency-free mutation fuzzing for parser robustness tests (plan 5.2).
//!
//! Included by integration tests with `#[path = "../../../testing/mutate.rs"] mod mutate;`.
//! Each target starts from valid seed inputs (fixtures or freshly exported files), applies
//! a few random byte and token mutations, and runs a property on the result. Runs are
//! deterministic: a failure prints the seed and iteration needed to reproduce it.
//!
//! - `SQUIB_FUZZ_ITERS` sets iterations per target (default: a short CI-sized run).
//! - `SQUIB_FUZZ_SEED` sets the base seed (default: fixed, so CI is repeatable).
//!
//! A longer local run: `SQUIB_FUZZ_ITERS=20000 SQUIB_FUZZ_SEED=$RANDOM cargo test --release robust`.

#![allow(dead_code)]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

/// xorshift64*: small, fast, and good enough to drive mutations.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform in `0..n` (n > 0).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    pub fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }
}

pub fn iterations(default: usize) -> usize {
    std::env::var("SQUIB_FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

pub fn base_seed() -> u64 {
    std::env::var("SQUIB_FUZZ_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(0x0051_551B)
}

/// Tokens that tend to break text and JSON parsers.
pub const JSON_TOKENS: &[&[u8]] = &[
    b"null",
    b"true",
    b"false",
    b"[]",
    b"{}",
    b"\"\"",
    b"0",
    b"-0",
    b"-1",
    b"1e309",
    b"-1e309",
    b"1e-400",
    b"NaN",
    b"Infinity",
    b"18446744073709551616",
    b"-9223372036854775809",
    b"9223372036854775807",
    b"4294967296",
    b"0.000000000000000000001",
    b"\"\\u0000\"",
    b"\"\\ud800\"",
    b"\"\xff\xfe\"",
    b"[[[[[[[[",
    b"]]]]",
    b"{\"a\":",
    b",",
    b":",
    b"\"",
    b"\\",
    b"\n",
    b"\r\n",
    b"\t",
    b" ",
    b"/",
    b"..",
    b"../",
    b"%00",
    b"\xe2\x80\xae",
    b"\xf0\x9f\x92\xa5",
];

/// Byte values on boundaries.
const INTERESTING: &[u8] = &[0x00, 0x01, 0x7f, 0x80, 0xff, b'0', b'9', b'-', b'.', b'e', b'"', b'{', b'['];

/// Apply one to four stacked mutations to a copy of `seed`.
pub fn mutate(rng: &mut Rng, seed: &[u8], tokens: &[&[u8]]) -> Vec<u8> {
    let mut v = seed.to_vec();
    for _ in 0..1 + rng.below(4) {
        let len = v.len();
        match rng.below(11) {
            // Flip one bit.
            0 if len > 0 => {
                let i = rng.below(len);
                v[i] ^= 1 << rng.below(8);
            }
            // Set a byte to a boundary value.
            1 if len > 0 => {
                let i = rng.below(len);
                v[i] = INTERESTING[rng.below(INTERESTING.len())];
            }
            // Random byte.
            2 if len > 0 => {
                let i = rng.below(len);
                v[i] = rng.next() as u8;
            }
            // Insert a token.
            3 if !tokens.is_empty() => {
                let t = tokens[rng.below(tokens.len())];
                let i = rng.below(len + 1);
                v.splice(i..i, t.iter().copied());
            }
            // Replace a run with a token.
            4 if len > 0 && !tokens.is_empty() => {
                let t = tokens[rng.below(tokens.len())];
                let i = rng.below(len);
                let j = (i + 1 + rng.below(16)).min(len);
                v.splice(i..j, t.iter().copied());
            }
            // Delete a range.
            5 if len > 0 => {
                let i = rng.below(len);
                let j = (i + 1 + rng.below(64)).min(len);
                v.drain(i..j);
            }
            // Duplicate a range in place.
            6 if len > 0 => {
                let i = rng.below(len);
                let j = (i + 1 + rng.below(64)).min(len);
                let chunk: Vec<u8> = v[i..j].to_vec();
                v.splice(j..j, chunk);
            }
            // Truncate.
            7 if len > 0 => v.truncate(rng.below(len)),
            // Overwrite a run with a constant.
            8 if len > 0 => {
                let i = rng.below(len);
                let j = (i + 1 + rng.below(32)).min(len);
                let b = INTERESTING[rng.below(INTERESTING.len())];
                v[i..j].fill(b);
            }
            // Swap two ranges of equal length.
            9 if len > 8 => {
                let n = 1 + rng.below(len / 4);
                let a = rng.below(len - n);
                let b = rng.below(len - n);
                for k in 0..n {
                    v.swap(a + k, b + k);
                }
            }
            // Append random bytes.
            _ => {
                for _ in 0..1 + rng.below(16) {
                    v.push(rng.next() as u8);
                }
            }
        }
    }
    v
}

fn preview(b: &[u8]) -> String {
    let s = String::from_utf8_lossy(&b[..b.len().min(400)]).escape_debug().to_string();
    if b.len() > 400 { format!("{s}... ({} bytes)", b.len()) } else { s }
}

/// Run `property` on `iters` mutants of the seeds. The property returns `Err` for a
/// violated invariant; a panic counts as a failure too. Each input must also finish
/// within `per_input`, which catches runaway loops and quadratic blowups.
pub fn fuzz<F>(name: &str, seeds: &[Vec<u8>], tokens: &[&[u8]], iters: usize, per_input: Duration, mut property: F)
where
    F: FnMut(&[u8]) -> Result<(), String>,
{
    assert!(!seeds.is_empty(), "{name}: no seeds");
    let base = base_seed();
    // Seeds themselves first, unmutated.
    let mut cases: Vec<(usize, Vec<u8>)> = seeds.iter().cloned().enumerate().map(|(i, s)| (usize::MAX - i, s)).collect();
    let mut rng = Rng::new(base ^ fxhash(name));
    for i in 0..iters {
        let s = &seeds[rng.below(seeds.len())];
        cases.push((i, mutate(&mut rng, s, tokens)));
    }
    let mut failure = None;
    for (i, input) in cases {
        let t = Instant::now();
        let r = catch_unwind(AssertUnwindSafe(|| property(&input)));
        let took = t.elapsed();
        let msg = match r {
            Ok(Ok(())) if took <= per_input => continue,
            Ok(Ok(())) => format!("took {took:?}, limit {per_input:?}"),
            Ok(Err(e)) => e,
            Err(p) => {
                let m = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()));
                format!("panicked: {}", m.unwrap_or_default())
            }
        };
        let which = if i > usize::MAX / 2 { format!("seed input {}", usize::MAX - i) } else { format!("iteration {i}") };
        failure = Some(format!("{name}: {msg}\n  reproduce with SQUIB_FUZZ_SEED={base} ({which})\n  input: {}", preview(&input)));
        break;
    }
    if let Some(f) = failure {
        panic!("{f}");
    }
}

fn fxhash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

/// `depth` levels of nested arrays: deeper than any parser should recurse.
pub fn deeply_nested(depth: usize) -> Vec<u8> {
    let mut v = vec![b'['; depth];
    v.extend(std::iter::repeat_n(b']', depth));
    v
}
