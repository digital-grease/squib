//! End-to-end engine flows with a simulated platform adapter.
//!
//! The simulator plays the role of the Android shell: it reacts to native effects,
//! "renders" cues into the simulated microphone stream at a known frame, adds impulses
//! at known offsets, and pushes PCM through the same queue/worker path as JNI.
//! Desktop-tested only: this proves engine logic, not any device's acoustics.

use std::sync::Arc;
use std::time::Duration;

use squib_domain::{CueKind, frames_to_ns};
use squib_mobile::*;
use squib_timing::clock::FrameAnchor;
use squib_timing::cue::cue_template;
use squib_timing::synth::Rng;

const RATE: u32 = 48_000;
const BLOCK: usize = 480;
const BASE_NS: i64 = 5_000_000_000_000;

fn db_path(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("squib-engine-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.join("journal.db").to_string_lossy().into()
}

fn route() -> RouteReport {
    RouteReport {
        input_device: "builtin_mic".into(),
        output_device: "builtin_speaker".into(),
        audio_source: "unprocessed".into(),
        unprocessed_supported: true,
        effects: vec![],
        os_build: "sim".into(),
        device_model: "simulator".into(),
        media_volume: 0.8,
        mic_permission: true,
    }
}

struct Sim {
    eng: Arc<SquibEngine>,
    frame: i64,
    handle: u64,
    seq: u64,
    rng: Rng,
    inject: Vec<(i64, Vec<f32>)>,
    shot_offsets_ms: Vec<f64>,
    cue_audible: bool,
    shot_frames: Vec<i64>,
    start_cue_frame: Option<i64>,
    drop_block_at: Option<u64>,
    capture_route: RouteReport,
    pub views: Vec<EngineView>,
    utc: i64,
}

impl Sim {
    fn new(name: &str) -> Self {
        let eng = SquibEngine::new(db_path(name), 1_700_000_000_000).unwrap();
        Self {
            eng,
            frame: 0,
            handle: 0,
            seq: 0,
            rng: Rng::new(42),
            inject: vec![],
            shot_offsets_ms: vec![],
            cue_audible: true,
            shot_frames: vec![],
            start_cue_frame: None,
            drop_block_at: None,
            capture_route: route(),
            views: vec![],
            utc: 1_700_000_000_000,
        }
    }

    fn now(&self) -> i64 {
        BASE_NS + frames_to_ns(self.frame, RATE)
    }

    fn arm(&mut self, mode: Mode, delay: StartDelay, pars: Vec<u32>, expected: Option<u32>) -> EngineView {
        let req = ArmRequest {
            mode,
            delay,
            unit_random: 0.5,
            pars_ms: pars,
            auto_stop_grace_ms: None,
            expected_count: expected,
            threshold_db: None,
            calibration_id: None,
            route: Some(route()),
            now_utc_ms: self.utc,
            tz_offset_min: -300,
            app_build: "test".into(),
        };
        let v = self.eng.arm(format!("arm-{}", self.utc), req, self.now()).unwrap();
        self.handle_effects(&v.effects.clone());
        v
    }

    fn handle_effects(&mut self, fx: &[NativeEffect]) {
        for e in fx {
            match e {
                NativeEffect::StartCapture { .. } => {
                    let meta = CaptureMeta {
                        sample_rate_hz: RATE,
                        block_frames: BLOCK as u32,
                        platform_buffer_frames: Some(4 * BLOCK as u32),
                        route: self.capture_route.clone(),
                        clock_domain: "CLOCK_MONOTONIC".into(),
                        started_mono_ns: self.now(),
                    };
                    self.frame = 0;
                    self.seq = 0;
                    self.handle = self.eng.capture_started(meta, self.now()).unwrap();
                }
                NativeEffect::StopCapture => self.handle = 0,
                NativeEffect::PlayCue { cue_id, cue } => {
                    // Simulated output latency: cue reaches the mic 12 ms after now;
                    // platform render timestamp reports 10 ms (2 ms acoustic path).
                    let render_frame = self.frame + (RATE as i64) * 10 / 1000;
                    let mic_frame = self.frame + (RATE as i64) * 12 / 1000;
                    let kind: CueKind = (*cue).into();
                    if self.cue_audible {
                        let t: Vec<f32> = cue_template(kind, RATE).iter().map(|v| v * 0.1).collect();
                        self.inject.push((mic_frame, t));
                    }
                    if kind == CueKind::Start {
                        self.start_cue_frame = Some(mic_frame);
                        for &ms in &self.shot_offsets_ms.clone() {
                            let f = mic_frame + (ms * f64::from(RATE) / 1000.0) as i64;
                            self.shot_frames.push(f);
                            self.inject.push((f, shot(&mut self.rng)));
                        }
                    }
                    self.eng.cue_rendered(*cue_id, Some(BASE_NS + frames_to_ns(render_frame, RATE)), self.now());
                }
                NativeEffect::KeepScreenOn { .. } => {}
            }
        }
    }

    /// Advance simulated time by `ms`, pushing audio when capture is open.
    fn run_ms(&mut self, ms: i64) -> EngineView {
        let blocks = ms * (RATE as i64) / 1000 / BLOCK as i64;
        let mut last = None;
        for _ in 0..blocks {
            if self.handle != 0 {
                let mut buf = vec![0f32; BLOCK];
                for x in buf.iter_mut() {
                    *x = (self.rng.gaussian() * 0.0018) as f32; // ≈ −55 dBFS
                }
                for (start, sig) in &self.inject {
                    for (i, x) in buf.iter_mut().enumerate() {
                        let f = self.frame + i as i64;
                        if f >= *start && f < start + sig.len() as i64 {
                            *x += sig[(f - start) as usize];
                        }
                    }
                }
                let pcm: Vec<i16> = buf.iter().map(|x| (x.clamp(-1.0, 1.0) * 32767.0) as i16).collect();
                let end = self.frame + BLOCK as i64;
                let anchor =
                    self.seq.is_multiple_of(10).then_some(FrameAnchor { frame: end, mono_ns: BASE_NS + frames_to_ns(end, RATE) });
                let delivered = BASE_NS + frames_to_ns(end, RATE) + 3_000_000;
                if self.drop_block_at != Some(self.seq) {
                    let code = data_plane_push(self.handle, self.seq, self.frame, &pcm, anchor, Some(delivered), 0);
                    assert!(code == 0 || code == 3, "push code {code}");
                    // After an injected gap the epoch is failed and processing stops by design.
                    let after_gap = self.drop_block_at.is_some_and(|d| self.seq > d);
                    if code == 0 && !after_gap {
                        self.wait_processed(end);
                    }
                }
                self.seq += 1;
            }
            self.frame += BLOCK as i64;
            let v = self.eng.poll(self.now());
            self.handle_effects(&v.effects.clone());
            last = Some(v.clone());
            self.views.push(v);
        }
        last.unwrap_or_else(|| self.eng.poll(self.now()))
    }

    fn wait_processed(&self, end: i64) {
        for _ in 0..50_000 {
            if self.eng.capture_diagnostics().is_none_or(|d| d.processed_frames >= end) {
                return;
            }
            std::thread::sleep(Duration::from_micros(100));
        }
        panic!("DSP worker did not process frame {end}");
    }

    fn settle(&mut self, phase: &str) -> EngineView {
        for _ in 0..300 {
            // Storage acknowledges asynchronously in real time; give it a moment.
            std::thread::sleep(Duration::from_millis(1));
            let v = self.run_ms(10);
            if v.phase == phase {
                return v;
            }
        }
        panic!(
            "never reached {phase}; last {:?}",
            self.views.last().map(|v| (&v.phase, &v.error, &v.interrupt_reason, &v.persist, v.durable_seq, &v.quality_labels))
        );
    }
}

fn shot(rng: &mut Rng) -> Vec<f32> {
    let n = (RATE as usize) * 64 / 1000;
    (0..n)
        .map(|i| {
            let env = if i < 5 { (i as f32 + 1.0) / 5.0 } else { (-(i as f32 - 5.0) / (0.008 * RATE as f32)).exp() };
            let c = 2.0 * rng.unit() as f32 - 1.0;
            0.8 * env * c.signum() * (0.5 + 0.5 * c.abs())
        })
        .collect()
}

#[test]
fn a01_a02_par_only_runs_offline_without_microphone() {
    let mut s = Sim::new("paronly");
    let pf = s.eng.preflight(Mode::ParOnly, RouteReport { mic_permission: false, ..route() }, RATE);
    assert!(pf.can_arm, "{pf:?}");
    let v = s.arm(Mode::ParOnly, StartDelay::Fixed { ms: 1000 }, vec![1000, 2000], None);
    assert_eq!(v.phase, "armed");
    assert!(!v.effects.iter().any(|e| matches!(e, NativeEffect::StartCapture { .. })), "no microphone");
    s.settle("running");
    let v = s.views.iter().rev().find(|v| v.phase == "running").unwrap();
    assert_eq!(v.start_method.as_deref(), Some("scheduled_render"));
    s.run_ms(2300);
    let v = s.eng.stop("stop-1".into(), s.now()).unwrap();
    assert_ne!(v.persist.as_deref(), Some("saved"), "saved only after durable ack");
    let v = s.settle("saved");
    assert_eq!(v.outcome.as_deref(), Some("complete"));
    assert_eq!(v.pars_played, 2);
    let runs = s.eng.list_runs(10).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].mode, "par_only");
    assert_eq!(runs[0].start_method.as_deref(), Some("scheduled_render"));
    let r = s.eng.load_review(runs[0].run_id.clone()).unwrap();
    assert_eq!(r.count, 0);
    assert!(r.energy_db.is_empty(), "no audio captured in par-only");
}

#[test]
fn phone_live_end_to_end_review_and_repeat() {
    let mut s = Sim::new("live");
    s.shot_offsets_ms = vec![700.0, 1000.0, 1350.0];
    let pf = s.eng.preflight(Mode::PhoneLive, route(), RATE);
    assert!(pf.can_arm);
    assert_eq!(pf.level, RouteLevel::Experimental);
    let v = s.arm(Mode::PhoneLive, StartDelay::Random { min_ms: 800, max_ms: 1200 }, vec![], Some(4));
    assert_eq!(v.phase, "preparing", "not armed before capture and baseline");
    s.settle("running");
    s.run_ms(2000);
    let v = s.eng.poll(s.now());
    assert_eq!(v.start_method.as_deref(), Some("acoustic_cue"));
    assert_eq!(v.accepted_count, 3, "{v:?}");
    s.eng.stop("stop-1".into(), s.now()).unwrap();
    let v = s.settle("saved");
    assert_eq!(v.outcome.as_deref(), Some("complete"));

    let run_id = v.run_id.clone().unwrap();
    let r = s.eng.load_review(run_id.clone()).unwrap();
    assert_eq!(r.count, 3);
    assert_eq!(r.expected_count, Some(4), "hint shown, never forced");
    assert_eq!(r.timestamp_quality.as_deref(), Some("verified_sample_clock"));
    let tol = 300_000; // 0.3 ms
    let first = r.first_ns.unwrap();
    assert!((first - 700_000_000).abs() <= tol, "first {first}");
    assert_eq!(r.splits_ns.len(), 2);
    assert!((r.splits_ns[0] - 300_000_000).abs() <= tol);
    assert!((r.splits_ns[1] - 350_000_000).abs() <= tol);
    assert!(!r.energy_db.is_empty());
    let originals: Vec<_> = r.events.iter().map(|e| (e.reference.clone(), e.timeline_ns)).collect();

    // A07: add a missed shot, move one, reject one -> new revision; originals preserved.
    let second = r.events.iter().find(|e| e.state == "accepted" && (e.timeline_ns - 1_000_000_000).abs() < 1_000_000).unwrap();
    let third = r.events.iter().find(|e| e.state == "accepted" && (e.timeline_ns - 1_350_000_000).abs() < 1_000_000).unwrap();
    let r2 = s
        .eng
        .apply_review(
            run_id.clone(),
            r.revision_number,
            vec![
                ReviewAction::AddManual { timeline_ns: 1_800_000_000 },
                ReviewAction::Move { reference: second.reference.clone(), timeline_ns: 990_000_000 },
                ReviewAction::Reject { sequence: third.reference.sequence.unwrap() },
            ],
            Some("test".into()),
            s.utc + 1,
        )
        .unwrap();
    assert_eq!(r2.revision_number, r.revision_number + 1);
    assert!(r2.edited);
    assert_eq!(r2.review_state.as_deref(), Some("reviewed"));
    assert_eq!(r2.count, 3);
    assert!(r2.events.iter().any(|e| e.origin == "manual"));
    assert!(r2.events.iter().any(|e| e.origin == "moved"));
    for (reference, t) in originals {
        if reference.kind == "candidate" {
            assert!(r2.events.iter().any(|e| e.reference == reference), "original candidate kept");
            let _ = t;
        }
    }
    // Stale edit is refused, not merged silently.
    assert!(
        s.eng.apply_review(run_id.clone(), r.revision_number, vec![ReviewAction::Note { text: "x".into() }], None, 0).is_err()
    );

    // Repeat: a new run with a new id.
    s.utc += 10;
    let v = s.arm(Mode::PhoneLive, StartDelay::Instant, vec![], None);
    assert_ne!(v.run_id.as_deref(), Some(run_id.as_str()));
}

#[test]
fn a04_inaudible_cue_interrupts_without_reaction_time() {
    let mut s = Sim::new("nocue");
    s.cue_audible = false;
    s.shot_offsets_ms = vec![150.0, 600.0]; // impulses exist, cue does not
    s.arm(Mode::PhoneLive, StartDelay::Instant, vec![], None);
    let v = s.settle("saved");
    assert_eq!(v.outcome.as_deref(), Some("interrupted"));
    let r = s.eng.load_review(v.run_id.unwrap()).unwrap();
    assert_eq!(r.first_ns, None, "no reaction time without an acoustic start");
    assert!(!r.origin_resolved);
    assert!(r.quality.iter().any(|q| q.label == "Start cue not heard"));
}

#[test]
fn a06_capture_gap_interrupts_and_is_recorded() {
    let mut s = Sim::new("gap");
    s.shot_offsets_ms = vec![500.0, 1500.0];
    s.arm(Mode::PhoneLive, StartDelay::Instant, vec![], None);
    s.settle("running");
    s.drop_block_at = Some(s.seq + 40);
    let v = s.settle("saved");
    assert_eq!(v.outcome.as_deref(), Some("interrupted"));
    assert_eq!(v.interrupt_reason.as_deref(), Some("Capture gap"));
    let r = s.eng.load_review(v.run_id.unwrap()).unwrap();
    assert!(r.quality.iter().any(|q| q.label == "Capture gap" && q.severity == "integrity"));
}

#[test]
fn a05_route_differing_from_preflight_is_not_silently_used() {
    let mut s = Sim::new("route");
    s.capture_route = RouteReport { input_device: "bluetooth_sco".into(), ..route() };
    s.arm(Mode::PhoneLive, StartDelay::Instant, vec![], None);
    let v = s.eng.poll(s.now());
    assert_eq!(v.phase, "failed", "{v:?}");
    let pf = s.eng.preflight(Mode::PhoneLive, RouteReport { output_device: "bluetooth_a2dp".into(), ..route() }, RATE);
    assert!(!pf.can_arm);
    assert_eq!(pf.level, RouteLevel::Unsupported);
    let pf = s.eng.preflight(Mode::PhoneLive, RouteReport { media_volume: 0.0, ..route() }, RATE);
    assert!(!pf.can_arm, "muted cue cannot be heard; volume is never overridden");
    let pf = s.eng.preflight(Mode::PhoneLive, RouteReport { mic_permission: false, ..route() }, RATE);
    assert!(!pf.can_arm && pf.messages[0].contains("Par-only"));
}

#[test]
fn lifecycle_loss_interrupts() {
    let mut s = Sim::new("lifecycle");
    s.arm(Mode::PhoneLive, StartDelay::Instant, vec![], None);
    s.settle("running");
    let v = s.eng.lifecycle(LifecycleEvent::MicSilenced, s.now());
    assert_eq!(v.phase, "interrupted");
    let v = s.settle("saved");
    assert_eq!(v.outcome.as_deref(), Some("interrupted"));
}

#[test]
fn a08_storage_failure_is_explicit_then_retry_saves() {
    let mut s = Sim::new("storagefail");
    s.arm(Mode::ParOnly, StartDelay::Instant, vec![], None);
    s.settle("running");
    s.eng.debug_inject_storage_faults(1);
    s.eng.stop("stop".into(), s.now()).unwrap();
    let v = s.settle("save_pending");
    assert_eq!(v.persist.as_deref(), Some("failed"));
    assert!(s.eng.reset().is_err(), "unsaved run cannot be silently abandoned");
    s.eng.retry_save("retry".into(), s.now()).unwrap();
    let v = s.settle("saved");
    assert_eq!(v.outcome.as_deref(), Some("complete"));
    assert_eq!(s.eng.list_runs(5).unwrap()[0].persist, "saved");
}

#[test]
fn intent_failure_blocks_arm_without_hidden_countdown() {
    let mut s = Sim::new("intentfail");
    s.eng.debug_inject_storage_faults(1);
    let v = s.arm(Mode::ParOnly, StartDelay::Instant, vec![], None);
    assert_eq!(v.phase, "failed");
    let v = s.run_ms(500);
    assert!(!v.effects.iter().any(|e| matches!(e, NativeEffect::PlayCue { .. })));
}

#[test]
fn cancel_before_cue_records_attempt_only() {
    let mut s = Sim::new("cancel");
    s.arm(Mode::ParOnly, StartDelay::Fixed { ms: 5000 }, vec![], None);
    s.run_ms(200);
    s.eng.cancel("c".into(), s.now()).unwrap();
    let v = s.settle("cancelled");
    assert_eq!(v.outcome.as_deref(), Some("cancelled"));
    s.run_ms(100);
    assert_eq!(s.eng.list_runs(5).unwrap()[0].outcome.as_deref(), Some("cancelled"));
}

#[test]
fn cue_test_reports_heard_and_not_heard() {
    let mut s = Sim::new("cuetest");
    let v = s.eng.start_cue_test(route(), s.now()).unwrap();
    s.handle_effects(&v.effects);
    let mut stage = String::new();
    for _ in 0..200 {
        s.run_ms(10);
        let v = s.eng.probe_status(s.now());
        s.handle_effects(&v.effects);
        stage = v.stage.clone();
        if stage == "done" {
            assert!(v.message.starts_with("Cue heard"), "{}", v.message);
            break;
        }
    }
    assert_eq!(stage, "done");
    s.eng.finish_probe(false, None, s.utc, s.now()).unwrap();
}

#[test]
fn guided_calibration_saves_route_scoped_profile() {
    let mut s = Sim::new("calib");
    let v = s.eng.start_calibration(route(), s.now()).unwrap();
    s.handle_effects(&v.effects);
    for _ in 0..350 {
        s.run_ms(10);
        let v = s.eng.probe_status(s.now());
        if v.stage == "impulses" {
            break;
        }
    }
    // Two test impulses after the ambient phase.
    let f = s.frame + 4800;
    let a = shot(&mut s.rng);
    let b = shot(&mut s.rng);
    s.inject.push((f, a));
    s.inject.push((f + 24_000, b));
    s.run_ms(1200);
    let v = s.eng.finish_probe(true, Some("Indoor test".into()), s.utc, s.now()).unwrap();
    assert_eq!(v.stage, "done", "{v:?}");
    assert_eq!(v.impulse_ratios_db.len(), 2, "{v:?}");
    assert_eq!(v.verdict.as_deref(), Some("separated"));
    let pf = s.eng.preflight(Mode::PhoneLive, route(), RATE);
    assert!(pf.calibration_id.is_some());
    assert_eq!(pf.calibration_label.as_deref(), Some("Indoor test"));
    let pf = s.eng.preflight(Mode::PhoneLive, route(), 44_100);
    assert!(pf.calibration_id.is_none(), "different rate invalidates calibration");
}

#[test]
fn a18_no_raw_pcm_is_written() {
    let mut s = Sim::new("nopcm");
    s.shot_offsets_ms = vec![500.0];
    s.arm(Mode::PhoneLive, StartDelay::Instant, vec![], None);
    s.settle("running");
    s.run_ms(3000);
    s.eng.stop("s".into(), s.now()).unwrap();
    s.settle("saved");
    let dir = std::path::Path::new(&db_path_existing("nopcm")).parent().unwrap().to_path_buf();
    let mut total = 0u64;
    for e in std::fs::read_dir(&dir).unwrap() {
        let e = e.unwrap();
        let name = e.file_name().to_string_lossy().to_string();
        assert!(!name.ends_with(".wav") && !name.ends_with(".pcm") && !name.ends_with(".raw"), "{name}");
        total += e.metadata().unwrap().len();
    }
    // ~4 s of PCM16 at 48 kHz would be 384 KB; the whole journal is far smaller.
    let pcm_bytes = 4 * 48_000 * 2;
    assert!(total < pcm_bytes as u64, "journal {total} bytes");
}

fn db_path_existing(name: &str) -> String {
    std::env::temp_dir().join(format!("squib-engine-{name}-{}", std::process::id())).join("journal.db").to_string_lossy().into()
}
