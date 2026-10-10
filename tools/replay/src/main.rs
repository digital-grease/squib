//! `squib-replay`: deterministic desktop replay through the production timing engine.
//!
//! ```text
//! squib-replay gen <dir>                       write golden synthetic WAV + label fixtures
//! squib-replay run <wav> [--labels <json>] [--chunk N | --random-chunks SEED]
//!                        [--start-ref-ms MS] [--threshold-db DB]
//! squib-replay corpus <dir>                    evaluate every fixture, check chunk invariance
//! squib-replay bench [--seconds S]             DSP per-block cost and queue transfer cost
//! ```
//! Output is JSON on stdout. Exit status is non-zero when a corpus check fails.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde::Serialize;
use squib_domain::{DetectorConfig, DetectorSuggestion};
use squib_timing::replay::{Chunking, ReplayOptions, ReplayOutput, evaluate, replay, replay_with, to_pcm16};
use squib_timing::synth::{SynthLabels, corpus, render};

fn sha256_file(p: &Path) -> String {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(p).expect("read file");
    Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn write_wav(path: &Path, rate: u32, pcm: &[i16]) {
    let spec = hound::WavSpec { channels: 1, sample_rate: rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(path, spec).expect("create wav");
    for &s in pcm {
        w.write_sample(s).expect("write sample");
    }
    w.finalize().expect("finalize wav");
}

fn read_wav(path: &Path) -> Result<(u32, Vec<i16>), String> {
    let mut r = hound::WavReader::open(path).map_err(|e| e.to_string())?;
    let spec = r.spec();
    if spec.channels != 1 || spec.bits_per_sample != 16 || spec.sample_format != hound::SampleFormat::Int {
        return Err(format!("expected mono PCM16, got {spec:?}"));
    }
    let s: Result<Vec<i16>, _> = r.samples::<i16>().collect();
    Ok((spec.sample_rate, s.map_err(|e| e.to_string())?))
}

#[derive(Serialize)]
struct ManifestEntry {
    name: String,
    wav: String,
    wav_sha256: String,
    labels: String,
    labels_sha256: String,
    description: String,
}

fn generate(dir: &Path) {
    std::fs::create_dir_all(dir).expect("create dir");
    let mut entries = Vec::new();
    for spec in corpus() {
        let (s, labels) = render(&spec);
        let pcm = to_pcm16(&s);
        let wav = dir.join(format!("{}.wav", spec.name));
        let lab = dir.join(format!("{}.labels.json", spec.name));
        write_wav(&wav, spec.sample_rate_hz, &pcm);
        std::fs::write(&lab, serde_json::to_string_pretty(&labels).unwrap() + "\n").unwrap();
        entries.push(ManifestEntry {
            name: spec.name.clone(),
            wav: wav.file_name().unwrap().to_string_lossy().into(),
            wav_sha256: sha256_file(&wav),
            labels: lab.file_name().unwrap().to_string_lossy().into(),
            labels_sha256: sha256_file(&lab),
            description: spec.description.clone(),
        });
    }
    let m = serde_json::json!({
        "generator": "squib-replay gen",
        "note": "Synthetic fixtures probe algorithm invariants only; they are not field evidence.",
        "fixtures": entries,
    });
    std::fs::write(dir.join("manifest.json"), serde_json::to_string_pretty(&m).unwrap() + "\n").unwrap();
    eprintln!("wrote {} fixtures to {}", corpus().len(), dir.display());
}

#[derive(Serialize)]
struct RunReport<'a> {
    sample_rate_hz: u32,
    frames: usize,
    output: &'a ReplayOutput,
    metrics: Option<squib_timing::replay::Metrics>,
}

fn opts_for(labels: Option<&SynthLabels>, rate: u32, start_ref_ms: Option<i64>, det: DetectorConfig) -> ReplayOptions {
    let o = ReplayOptions::new(det);
    match labels {
        Some(l) => o.with_label_cues(l, 20, start_ref_ms.or(Some(1000))),
        None => {
            let mut o = o;
            if let Some(ms) = start_ref_ms {
                o.cues.push(squib_timing::replay::CueReference {
                    cue_id: 0,
                    kind: squib_domain::CueKind::Start,
                    reference_frame: squib_domain::ns_to_frames(ms * 1_000_000, rate),
                    render_known: false,
                });
            }
            o
        }
    }
}

fn arg_value(args: &[String], key: &str) -> Option<String> {
    args.iter().position(|a| a == key).and_then(|i| args.get(i + 1).cloned())
}

fn cmd_run(args: &[String]) -> ExitCode {
    let Some(wav) = args.first() else {
        eprintln!("usage: squib-replay run <wav> [options]");
        return ExitCode::from(2);
    };
    let (rate, pcm) = match read_wav(Path::new(wav)) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let labels: Option<SynthLabels> = arg_value(args, "--labels")
        .map(|p| serde_json::from_str(&std::fs::read_to_string(p).expect("labels")).expect("labels json"));
    let mut det = DetectorConfig::default();
    if let Some(t) = arg_value(args, "--threshold-db") {
        det.threshold_db = t.parse().expect("threshold");
    }
    let mut o = opts_for(labels.as_ref(), rate, arg_value(args, "--start-ref-ms").map(|v| v.parse().expect("ms")), det);
    if let Some(n) = arg_value(args, "--chunk") {
        o.chunking = Chunking::Fixed { frames: n.parse().expect("chunk") };
    }
    if let Some(seed) = arg_value(args, "--random-chunks") {
        o.chunking = Chunking::Pseudorandom { seed: seed.parse().expect("seed"), min: 1, max: 2048 };
    }
    let out = replay(&pcm, rate, &o);
    let metrics = labels.as_ref().map(|l| evaluate(&out, l, i64::from(rate) / 4000));
    println!(
        "{}",
        serde_json::to_string_pretty(&RunReport { sample_rate_hz: rate, frames: pcm.len(), output: &out, metrics }).unwrap()
    );
    ExitCode::SUCCESS
}

fn cmd_corpus(dir: &Path) -> ExitCode {
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).expect("manifest")).expect("manifest json");
    let mut ok = true;
    let mut rows = Vec::new();
    for f in manifest["fixtures"].as_array().expect("fixtures") {
        let name = f["name"].as_str().unwrap();
        let wav = dir.join(f["wav"].as_str().unwrap());
        let lab = dir.join(f["labels"].as_str().unwrap());
        if sha256_file(&wav) != f["wav_sha256"].as_str().unwrap() || sha256_file(&lab) != f["labels_sha256"].as_str().unwrap() {
            eprintln!("{name}: fixture hash mismatch");
            ok = false;
            continue;
        }
        let (rate, pcm) = read_wav(&wav).expect("wav");
        let labels: SynthLabels = serde_json::from_str(&std::fs::read_to_string(&lab).unwrap()).unwrap();
        let base = opts_for(Some(&labels), rate, None, DetectorConfig::default());
        let a = replay(&pcm, rate, &base);
        let mut invariant = true;
        for ch in [
            Chunking::Fixed { frames: 1 },
            Chunking::Fixed { frames: 4096 },
            Chunking::Pseudorandom { seed: 11, min: 1, max: 3000 },
        ] {
            let mut o = base.clone();
            o.chunking = ch;
            let b = replay(&pcm, rate, &o);
            let on = |x: &ReplayOutput| x.classified.iter().map(|c| (c.onset_frame, c.classification)).collect::<Vec<_>>();
            invariant &= on(&a) == on(&b) && a.start_onset() == b.start_onset();
        }
        let m = evaluate(&a, &labels, i64::from(rate) / 4000);
        let max_err = m.onset_errors_frames.iter().map(|e| e.abs()).max().unwrap_or(0);
        let accepted = a.classified.iter().filter(|c| c.classification == DetectorSuggestion::SuggestedAccepted).count();
        let pass = invariant
            && m.missed_onsets.is_empty()
            && m.start_cue_found == m.start_cue_expected
            && m.unmatched_candidates.is_empty();
        ok &= pass;
        rows.push(serde_json::json!({
            "fixture": name,
            "pass": pass,
            "chunk_invariant": invariant,
            "labeled": m.labeled_events,
            "matched": m.matched,
            "suggested_accepted_total": accepted,
            "matched_suggested_accepted": m.matched_suggested_accepted,
            "unmatched_non_rejected": m.unmatched_candidates.len(),
            "max_abs_onset_error_frames": max_err,
            "start_cue_error_frames": m.start_cue_error_frames,
            "start_cue_expected": m.start_cue_expected,
            "start_cue_found": m.start_cue_found,
        }));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({"evidence_level": "desktop-tested (synthetic)", "results": rows}))
            .unwrap()
    );
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn cmd_bench(args: &[String]) -> ExitCode {
    let secs: f64 = arg_value(args, "--seconds").map(|v| v.parse().expect("seconds")).unwrap_or(60.0);
    let rate = 48_000u32;
    // Long stream: repeat the basic fixture's audio.
    let spec = corpus().into_iter().next().unwrap();
    let (s, _) = render(&spec);
    let one = to_pcm16(&s);
    let total = (secs * f64::from(rate)) as usize;
    let pcm: Vec<i16> = one.iter().cycle().take(total).copied().collect();
    let mut o = ReplayOptions::new(DetectorConfig::default());
    o.chunking = Chunking::Fixed { frames: 480 };
    let mut times: Vec<u64> = Vec::with_capacity(total / 480 + 1);
    let _ = replay_with(&pcm, rate, &o, |_, d| times.push(d.as_nanos() as u64));
    times.sort_unstable();
    let q = |p: f64| times[((times.len() - 1) as f64 * p).round() as usize];
    let block_ns = 480u64 * 1_000_000_000 / u64::from(rate);

    // Queue transfer cost (push + pop + recycle), the Rust side of the data plane.
    let queue = squib_mobile::queue::PcmQueue::for_duration(rate, 480, 250);
    let block = vec![0i16; 480];
    let n = 200_000u64;
    let t0 = std::time::Instant::now();
    for i in 0..n {
        let _ = queue.push(i, (i * 480) as i64, &block, None, None);
        let b = queue.pop().unwrap();
        queue.recycle(b);
    }
    let per_push = t0.elapsed().as_nanos() as u64 / n;
    let report = serde_json::json!({
        "evidence_level": "desktop-measured (host CPU, not a phone)",
        "host": std::env::consts::ARCH,
        "stream_seconds": secs,
        "block_frames": 480,
        "block_duration_ns": block_ns,
        "dsp_blocks": times.len(),
        "dsp_p50_ns": q(0.5),
        "dsp_p99_ns": q(0.99),
        "dsp_max_ns": times.last(),
        "budget_p99_ns": block_ns / 2,
        "within_budget_on_host": q(0.99) < block_ns / 2,
        "queue_push_pop_ns": per_push,
    });
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    ExitCode::SUCCESS
}

/// M5 experiment 3: an opt-in diagnostic export. Checks that replay reproduces the
/// phone's detections, then scores detector variants against the user's own review.
/// Scoring is at candidate level: a labelled shot is found when a candidate onset lies
/// within 0.25 ms of it; other candidates after the start and outside cue audio are false.
fn cmd_diag(args: &[String]) -> ExitCode {
    use squib_archive::diagnostic::read_diagnostic;
    use squib_timing::stress::{exploratory, variants};
    let Some(path) = args.first() else {
        eprintln!("usage: squib-replay diag <export.zip>");
        return ExitCode::from(2);
    };
    let b = match read_diagnostic(Path::new(path)) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read export: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut wav = match hound::WavReader::new(std::io::Cursor::new(&b.wav)) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("recording.wav is not a valid WAV: {e}");
            return ExitCode::FAILURE;
        }
    };
    let pcm: Vec<i16> = wav.samples::<i16>().map(|s| s.unwrap_or(0)).collect();
    let rate = b.recording.sample_rate_hz;
    let off = b.recording.first_epoch_frame;
    let Some(recorded) = b.config.detector.clone() else {
        eprintln!("export has no detector configuration");
        return ExitCode::FAILURE;
    };
    let run = |det: &DetectorConfig| replay(&pcm, rate, &ReplayOptions::new(det.clone()));
    let base = run(&recorded);
    let got: Vec<(i64, i64)> = base.candidates.iter().map(|c| (c.onset_frame + off, c.peak_frame + off)).collect();
    let want: Vec<(i64, i64)> = b.labels.candidates.iter().map(|c| (c.onset_frame, c.peak_frame)).collect();
    let exact = got == want;
    let shots: Vec<i64> = b.labels.accepted.iter().filter_map(|e| e.epoch_frame).collect();
    let tol = i64::from(rate) / 4000;
    let minutes = pcm.len() as f64 / f64::from(rate) / 60.0;
    let score = |out: &squib_timing::replay::ReplayOutput| {
        let onsets: Vec<i64> = out.candidates.iter().map(|c| c.onset_frame + off).collect();
        let mut used = vec![false; onsets.len()];
        let mut found = 0;
        for s in &shots {
            if let Some((i, _)) = onsets
                .iter()
                .enumerate()
                .filter(|(i, o)| !used[*i] && (**o - s).abs() <= tol)
                .min_by_key(|(_, o)| (**o - s).abs())
            {
                used[i] = true;
                found += 1;
            }
        }
        // As the app's classification does: candidates inside a heard cue, or before the
        // start reference, are cues or pre-start sounds, not false shots.
        let excluded = |o: i64| {
            b.labels.start_ref_frame.is_some_and(|st| o < st)
                || b.labels.cues.iter().any(|c| match (c.span_start_frame, c.span_end_frame) {
                    (Some(a), Some(e)) => (a..=e).contains(&o),
                    _ => false,
                })
        };
        let false_n = used.iter().zip(&onsets).filter(|(u, o)| !**u && !excluded(**o)).count();
        serde_json::json!({
            "recall": if shots.is_empty() { 1.0 } else { found as f64 / shots.len() as f64 },
            "found": found,
            "false_candidates": false_n,
            "false_per_min": false_n as f64 / minutes.max(1e-9),
        })
    };
    let mut rows = vec![serde_json::json!({"variant": "as_recorded", "score": score(&base)})];
    for (name, det) in variants().into_iter().chain(exploratory()) {
        rows.push(serde_json::json!({"variant": name, "score": score(&run(&det))}));
    }
    let report = serde_json::json!({
        "evidence_level": "single opt-in field recording (not a cohort)",
        "export_id": b.manifest.export_id,
        "consent_scope": b.manifest.consent.scope,
        "device": b.config.route.as_ref().map(|r| r.device_model.clone()),
        "seconds": pcm.len() as f64 / f64::from(rate),
        "gaps": b.recording.gaps.len(),
        "truncated": b.recording.truncated,
        "reproduces_phone_candidates": exact,
        "reproduction_note": if b.recording.gaps.is_empty() { "no gaps: exact reproduction expected" } else { "gaps present: frames were silenced, so exact reproduction is not expected" },
        "labelled_shots": shots.len(),
        "reviewed_by_person": b.labels.reviewed_by_person,
        "results": rows,
    });
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    if !exact && b.recording.gaps.is_empty() { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

/// M5 experiment 2: every pre-declared variant on every cohort, plus verdicts.
fn cmd_stress(args: &[String]) -> ExitCode {
    use squib_timing::stress::{COHORTS, exploratory, run_cohort, variants, verdict};
    let mut rows = Vec::new();
    for (name, det) in variants().into_iter().chain(exploratory()) {
        for c in COHORTS {
            rows.push(run_cohort(c, name, &det));
        }
    }
    let verdicts: Vec<_> = variants().iter().skip(1).map(|(n, _)| verdict(n, &rows)).collect();
    if args.iter().any(|a| a == "--markdown") {
        println!(
            "| cohort | variant | recall | onset p95 ms | false/min | distractors detected (accepted) | accepted false/min |"
        );
        println!("|---|---|---|---|---|---|---|");
        for r in &rows {
            println!(
                "| {} | {} | {:.2} | {:.3} | {:.1} | {}/{} ({}) | {:.1} |",
                r.cohort,
                r.variant,
                r.recall,
                r.onset_error_p95_ms,
                r.false_per_min,
                r.distractors_detected,
                r.distractors,
                r.distractors_accepted,
                r.accepted_false_per_min
            );
        }
        println!();
        for v in &verdicts {
            println!("- {}: {} ({})", v.variant, if v.go { "GO to field validation" } else { "no-go" }, v.reasons.join("; "));
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "evidence_level": "desktop-tested (synthetic stress cohorts)",
                "results": rows,
                "verdicts": verdicts,
            }))
            .unwrap()
        );
    }
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("gen") => {
            generate(&PathBuf::from(args.get(1).map(String::as_str).unwrap_or("fixtures/synthetic")));
            ExitCode::SUCCESS
        }
        Some("run") => cmd_run(&args[1..]),
        Some("corpus") => cmd_corpus(&PathBuf::from(args.get(1).map(String::as_str).unwrap_or("fixtures/synthetic"))),
        Some("bench") => cmd_bench(&args[1..]),
        Some("stress") => cmd_stress(&args[1..]),
        Some("diag") => cmd_diag(&args[1..]),
        _ => {
            eprintln!("usage: squib-replay <gen|run|corpus|bench|stress|diag> ...");
            ExitCode::from(2)
        }
    }
}
