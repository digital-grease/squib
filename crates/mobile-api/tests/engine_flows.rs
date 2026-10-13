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
        camera_recording: false,
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
    expected_rate: u32,
    pub views: Vec<EngineView>,
    utc: i64,
    diag_root: Option<String>,
    auto_stop_count: Option<u32>,
    cue_channel: CueChannel,
    /// Keep every view (most tests inspect them); long runs turn this off.
    record_views: bool,
    /// The analysis thread was made to panic: do not wait for it to process blocks.
    dsp_dead: bool,
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
            expected_rate: RATE,
            views: vec![],
            utc: 1_700_000_000_000,
            diag_root: None,
            auto_stop_count: None,
            cue_channel: CueChannel::Sound,
            record_views: true,
            dsp_dead: false,
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
            auto_stop_count: self.auto_stop_count,
            expected_count: expected,
            threshold_db: None,
            calibration_id: None,
            route: Some(route()),
            now_utc_ms: self.utc,
            tz_offset_min: -300,
            app_build: "test".into(),
            expected_rate_hz: self.expected_rate,
            drill_id: None,
            drill_version: None,
            plan_item_id: None,
            video: false,
            cue_channel: self.cue_channel,
            diagnostic_root: self.diag_root.clone(),
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
                NativeEffect::PlayCue { cue_id, channel: CueChannel::Vibration, .. } => {
                    // Vibration: the adapter reports the time it asked the vibrator to start.
                    self.eng.cue_rendered(*cue_id, Some(self.now()), self.now());
                }
                NativeEffect::PlayCue { cue_id, channel: CueChannel::Screen, .. } => {
                    // Screen: the frame time of the first frame showing the signal.
                    self.eng.cue_rendered(*cue_id, Some(self.now() + 16 * 1_000_000), self.now());
                }
                NativeEffect::PlayCue { cue_id, cue, channel: CueChannel::Sound } => {
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
                // Sounds that have finished playing are no longer needed.
                let frame = self.frame;
                self.inject.retain(|(start, sig)| start + sig.len() as i64 > frame);
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
                    if code == 0 && !after_gap && !self.dsp_dead {
                        self.wait_processed(end);
                    }
                }
                self.seq += 1;
            }
            self.frame += BLOCK as i64;
            let v = self.eng.poll(self.now());
            self.handle_effects(&v.effects.clone());
            last = Some(v.clone());
            if self.record_views {
                self.views.push(v);
            }
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

    fn settle_persisted(&mut self) -> EngineView {
        for _ in 0..1000 {
            std::thread::sleep(Duration::from_millis(2));
            let v = self.run_ms(10);
            if v.persist.as_deref() == Some("saved") {
                return v;
            }
        }
        panic!("run never durably saved; last {:?}", self.views.last().map(|v| (&v.phase, &v.persist)));
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
fn vibration_and_screen_cues_are_par_only_and_saved_as_their_own_start_methods() {
    for (ch, method) in [(CueChannel::Vibration, "vibration_request"), (CueChannel::Screen, "screen_frame")] {
        let mut s = Sim::new(&format!("cue-{method}"));
        s.cue_channel = ch;
        // Shot timing needs the sound cue it listens for.
        let live = s.eng.arm(
            "live".into(),
            ArmRequest {
                mode: Mode::PhoneLive,
                delay: StartDelay::Instant,
                unit_random: 0.0,
                pars_ms: vec![],
                auto_stop_grace_ms: None,
                auto_stop_count: None,
                expected_count: None,
                threshold_db: None,
                calibration_id: None,
                route: Some(route()),
                now_utc_ms: s.utc,
                tz_offset_min: 0,
                app_build: "test".into(),
                expected_rate_hz: RATE,
                drill_id: None,
                drill_version: None,
                plan_item_id: None,
                video: false,
                cue_channel: ch,
                diagnostic_root: None,
            },
            s.now(),
        );
        assert!(live.is_err(), "{ch:?} cues are par-only");

        s.arm(Mode::ParOnly, StartDelay::Fixed { ms: 500 }, vec![1000], None);
        s.settle("running");
        let v = s.views.iter().rev().find(|v| v.phase == "running").unwrap();
        assert_eq!(v.start_method.as_deref(), Some(method));
        s.run_ms(1300);
        s.eng.stop("stop".into(), s.now()).unwrap();
        let v = s.settle("saved");
        assert_eq!(v.pars_played, 1, "par cues use the same channel and still play");
        let runs = s.eng.list_runs(10).unwrap();
        assert_eq!(runs[0].start_method.as_deref(), Some(method));
        assert_eq!(s.eng.load_review(runs[0].run_id.clone()).unwrap().start_method.as_deref(), Some(method));
        assert!(s.eng.export_csv().unwrap().contains(method), "exports name the method");
    }
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
fn calibration_and_route_signature_use_expected_rate() {
    // A device that negotiates a different rate than expected fails safely at capture.
    let mut s = Sim::new("rate");
    s.expected_rate = 44_100;
    s.arm(Mode::PhoneLive, StartDelay::Instant, vec![], None);
    let v = s.eng.poll(s.now());
    assert_eq!(v.phase, "failed", "48 kHz capture must not run under a 44.1 kHz signature");
    // Matching expectation arms normally.
    let mut s = Sim::new("rate-ok");
    s.arm(Mode::PhoneLive, StartDelay::Instant, vec![], None);
    s.settle("running");
}

#[test]
fn audio_focus_loss_interrupts() {
    let mut s = Sim::new("focus");
    s.arm(Mode::ParOnly, StartDelay::Instant, vec![], None);
    s.settle("running");
    let v = s.eng.lifecycle(LifecycleEvent::AudioFocusLost, s.now());
    assert_eq!(v.phase, "interrupted");
    assert_eq!(v.interrupt_reason.as_deref(), Some("Audio focus lost"));
    let v = s.settle("saved");
    assert_eq!(v.outcome.as_deref(), Some("interrupted"));
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
    // The attempt record is written asynchronously; read it only after the durable ack.
    s.settle_persisted();
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
    let db = db_path_existing("nopcm");
    let dir = std::path::Path::new(&db).parent().unwrap().to_path_buf();
    // Only the journal and its WAL/SHM files exist: no audio files of any kind.
    for e in std::fs::read_dir(&dir).unwrap() {
        let name = e.unwrap().file_name().to_string_lossy().to_string();
        assert!(["journal.db", "journal.db-wal", "journal.db-shm"].contains(&name.as_str()), "unexpected file {name}");
    }
    // Inside the journal, the only bulk data is the coarse envelope: 2 bytes per
    // 10 ms hop. Raw PCM16 would be 960 bytes per 10 ms.
    let c = rusqlite::Connection::open(&db).unwrap();
    let captured_s = 4.0; // baseline + run, rounded up
    let envelope: i64 = c.query_row("SELECT COALESCE(SUM(length(data)), 0) FROM energy_envelope", [], |r| r.get(0)).unwrap();
    assert!(envelope > 0, "envelope stored");
    assert!(envelope as f64 <= captured_s * 100.0 * 2.0 + 200.0, "envelope {envelope} bytes exceeds coarse budget");
    let other_blobs: i64 = c.query_row("SELECT COALESCE(SUM(length(body)), 0) FROM provider_cache", [], |r| r.get(0)).unwrap();
    assert_eq!(other_blobs, 0, "no other bulk data written by a capture run");
}

fn db_path_existing(name: &str) -> String {
    std::env::temp_dir().join(format!("squib-engine-{name}-{}", std::process::id())).join("journal.db").to_string_lossy().into()
}

fn clip_meta(first_frame_ns: i64, probe_delta_ns: i64) -> VideoMeta {
    VideoMeta {
        mime: "video/mp4".into(),
        width: 1280,
        height: 720,
        frame_rate: 30.0,
        duration_ms: 60_000,
        first_frame_camera_ns: first_frame_ns,
        camera_clock: "unknown".into(),
        probe_camera_ns: first_frame_ns,
        probe_mono_ns: first_frame_ns + probe_delta_ns,
        probe_boot_ns: first_frame_ns + probe_delta_ns + 10_000_000_000,
        start_boot_minus_mono_ns: 10_000_000_000,
        end_boot_minus_mono_ns: 10_000_000_000,
    }
}

#[test]
fn video_markers_come_from_run_events_and_review_edits() {
    let mut s = Sim::new("video");
    s.shot_offsets_ms = vec![700.0, 1000.0, 1350.0];
    // Video changes the route: calibration from a non-video route does not apply.
    let pf = s.eng.preflight(Mode::PhoneLive, RouteReport { camera_recording: true, ..route() }, RATE);
    assert!(pf.messages.iter().any(|m| m.contains("Video is on")));
    assert_ne!(pf.route_signature, s.eng.preflight(Mode::PhoneLive, route(), RATE).route_signature);
    // Recording starts before arming, at simulated time zero.
    let first_frame = s.now();
    s.arm(Mode::PhoneLive, StartDelay::Random { min_ms: 800, max_ms: 1200 }, vec![], None);
    s.settle("running");
    s.run_ms(2000);
    s.eng.stop("stop".into(), s.now()).unwrap();
    let run_id = s.settle("saved").run_id.unwrap();

    let dir = std::env::temp_dir().join(format!("squib-video-att-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("videos")).unwrap();
    std::fs::write(dir.join("videos/a.mp4"), b"not really mp4").unwrap();
    let root = dir.to_string_lossy().to_string();
    let v =
        s.eng.register_video(run_id.clone(), root.clone(), "videos/a.mp4".into(), clip_meta(first_frame, 30_000_000), 5).unwrap();
    assert_eq!(v.mapping, "assumed", "unknown camera clock is never reported as measured");
    assert_eq!(v.uncertainty_ms, 33);
    let start = v.markers.iter().find(|m| m.kind == "start").unwrap();
    assert!(start.file_ms > 0);
    let shots: Vec<_> = v.markers.iter().filter(|m| m.kind == "shot").collect();
    assert_eq!(shots.len(), 3);
    for (m, want) in shots.iter().zip([700, 1000, 1350]) {
        assert!((m.run_ms - want).abs() <= 1, "{m:?}");
        assert_eq!(m.file_ms - start.file_ms, m.run_ms, "file position follows the run timeline");
        assert!(!m.edited && !m.approximate);
    }
    assert!(v.markers.windows(2).all(|w| w[0].file_ms <= w[1].file_ms));

    // A manual addition in review shows up as edited and approximate.
    let r = s.eng.load_review(run_id.clone()).unwrap();
    s.eng
        .apply_review(
            run_id.clone(),
            r.revision_number,
            vec![ReviewAction::AddManual { timeline_ns: 1_800_000_000 }],
            None,
            s.utc + 1,
        )
        .unwrap();
    let v = s.eng.run_videos(run_id.clone()).unwrap().remove(0);
    let manual = v.markers.iter().find(|m| m.run_ms == 1800).unwrap();
    assert!(manual.edited && manual.approximate);

    // Clocks that disagree give a playable clip without markers.
    std::fs::write(dir.join("videos/b.mp4"), b"x").unwrap();
    let bad =
        s.eng.register_video(run_id.clone(), root, "videos/b.mp4".into(), clip_meta(first_frame, 9_000_000_000), 6).unwrap();
    assert_eq!(bad.mapping, "unavailable");
    assert!(bad.markers.is_empty() && bad.mapping_note.starts_with("Not aligned"));

    // The problem report carries clock evidence, not the clip.
    let report = s.eng.diagnostic_report("d".into(), "a".into(), "v".into());
    assert!(report.contains("video | clock unknown | mapping assumed | camera minus callback clock -30.0 ms"), "{report}");
    assert!(!report.contains("videos/"));

    // Videos are not photos, and deleting the run removes both clips.
    assert!(s.eng.run_attachments(run_id.clone()).is_empty());
    let d = s.eng.delete_run(run_id).unwrap();
    assert_eq!(d.attachment_paths.len(), 2);
}

fn wav_pcm(bytes: &[u8]) -> Vec<i16> {
    assert_eq!(&bytes[0..4], b"RIFF");
    bytes[44..].chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect()
}

#[test]
fn diagnostic_recording_is_opt_in_exact_and_exports_only_with_consent() {
    let root = std::env::temp_dir().join(format!("squib-diag-flow-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let root_s = root.to_string_lossy().to_string();

    // Par-only cannot record (no microphone).
    let mut s = Sim::new("diag");
    s.diag_root = Some(root_s.clone());
    let req_err = s.eng.arm(
        "bad".into(),
        ArmRequest {
            mode: Mode::ParOnly,
            delay: StartDelay::Instant,
            unit_random: 0.0,
            pars_ms: vec![],
            auto_stop_grace_ms: None,
            auto_stop_count: None,
            expected_count: None,
            threshold_db: None,
            calibration_id: None,
            route: None,
            now_utc_ms: s.utc,
            tz_offset_min: 0,
            app_build: "test".into(),
            expected_rate_hz: RATE,
            drill_id: None,
            drill_version: None,
            plan_item_id: None,
            video: false,
            cue_channel: CueChannel::Sound,
            diagnostic_root: Some(root_s.clone()),
        },
        s.now(),
    );
    assert!(req_err.is_err(), "diagnostic recording needs a microphone mode");

    s.shot_offsets_ms = vec![700.0, 1000.0, 1350.0];
    s.arm(Mode::PhoneLive, StartDelay::Random { min_ms: 800, max_ms: 1200 }, vec![], Some(3));
    s.settle("running");
    s.run_ms(2000);
    s.eng.stop("stop".into(), s.now()).unwrap();
    let run_id = s.settle("saved").run_id.unwrap();
    // Registration happens off the engine lock after the durable save.
    let mut view = None;
    for _ in 0..500 {
        view = s.eng.run_diagnostic(run_id.clone()).unwrap();
        if view.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let view = view.expect("recording registered");
    assert_eq!((view.gaps, view.truncated, view.dropped_frames), (0, false, 0));
    assert_eq!(view.markers.iter().filter(|m| m.kind == "shot").count(), 3);
    assert!(view.markers.iter().any(|m| m.kind == "cue"), "start cue heard");
    let wav = std::fs::read(root.join(&view.relative_path)).unwrap();
    assert_eq!(wav.len() as i64, view.bytes);

    // Export: preview lists exclusions; refused without consent.
    let p = s.eng.diagnostic_preview(run_id.clone()).unwrap();
    assert!(p.excludes.iter().any(|x| x.contains("Location")));
    assert_eq!(p.accepted_events, 3);
    let out = root.join("export.zip").to_string_lossy().to_string();
    assert!(s.eng.export_diagnostic(run_id.clone(), root_s.clone(), out.clone(), false, "t".into(), s.utc).is_err());
    let e = s.eng.export_diagnostic(run_id.clone(), root_s.clone(), out.clone(), true, "t".into(), s.utc).unwrap();
    assert_eq!(e.accepted_events, 3);
    let b = squib_archive::diagnostic::read_diagnostic(std::path::Path::new(&out)).unwrap();
    assert_eq!(b.manifest.consent.scope, "research_only_no_redistribution");
    assert!(!b.manifest.contains_location);
    let text = String::from_utf8_lossy(&std::fs::read(&out).unwrap()).to_string();
    assert!(!text.contains(&run_id), "no link back to the run");

    // Success measure: replaying the exported audio reproduces the phone's detections exactly.
    let pcm = wav_pcm(&b.wav);
    let det = b.config.detector.clone().unwrap();
    let rep = squib_timing::replay::replay(&pcm, b.recording.sample_rate_hz, &squib_timing::replay::ReplayOptions::new(det));
    let off = b.recording.first_epoch_frame;
    let got: Vec<(i64, i64)> = rep.candidates.iter().map(|c| (c.onset_frame + off, c.peak_frame + off)).collect();
    let want: Vec<(i64, i64)> = b.labels.candidates.iter().map(|c| (c.onset_frame, c.peak_frame)).collect();
    assert!(!want.is_empty());
    assert_eq!(got, want, "exported recording reproduces the original candidates frame for frame");

    // Problem reports never carry the recording.
    assert!(!s.eng.diagnostic_report("d".into(), "a".into(), "v".into()).contains("diagnostics/"));
    // Start-up cleanup keeps owned recordings and removes strays.
    std::fs::write(root.join("diagnostics/stray.wav.partial"), b"x").unwrap();
    std::fs::write(root.join("diagnostics/orphan.wav"), b"x").unwrap();
    assert_eq!(s.eng.cleanup_diagnostic_partials(root_s.clone()), 2);
    assert!(root.join(&view.relative_path).exists());

    // Deleting the recording keeps the run.
    let path = s.eng.delete_diagnostic(run_id.clone()).unwrap().unwrap();
    assert_eq!(path, view.relative_path);
    assert!(s.eng.run_diagnostic(run_id.clone()).unwrap().is_none());
    assert!(s.eng.load_review(run_id).is_ok());
}

#[test]
fn cancelled_runs_keep_no_audio_and_default_runs_record_nothing() {
    let root = std::env::temp_dir().join(format!("squib-diag-cancel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let mut s = Sim::new("diag-cancel");
    s.diag_root = Some(root.to_string_lossy().to_string());
    s.arm(Mode::PhoneLive, StartDelay::Fixed { ms: 5000 }, vec![], None);
    s.settle("armed");
    s.run_ms(300);
    s.eng.cancel("c".into(), s.now()).unwrap();
    let run_id = s.settle("cancelled").run_id.unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert!(s.eng.run_diagnostic(run_id).unwrap().is_none());
    let left: Vec<_> = std::fs::read_dir(root.join("diagnostics")).map(|d| d.flatten().collect()).unwrap_or_default();
    assert!(left.is_empty(), "no audio file kept for a cancelled run: {left:?}");

    // Default (no opt-in): nothing is written at all.
    let root2 = std::env::temp_dir().join(format!("squib-diag-none-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root2);
    let mut s = Sim::new("diag-none");
    s.shot_offsets_ms = vec![700.0];
    s.arm(Mode::PhoneLive, StartDelay::Instant, vec![], None);
    s.settle("running");
    s.run_ms(1200);
    s.eng.stop("s".into(), s.now()).unwrap();
    let run_id = s.settle("saved").run_id.unwrap();
    assert!(s.eng.run_diagnostic(run_id).unwrap().is_none());
    assert!(!root2.exists());
}

#[test]
fn live_run_stops_itself_after_the_target_accepted_shot_count() {
    let mut s = Sim::new("count-stop");
    // Shots at 0.7, 1.0, 1.35 s; one more inside the 1 s grace; one long after.
    s.shot_offsets_ms = vec![700.0, 1000.0, 1350.0, 1800.0, 4000.0];
    s.auto_stop_count = Some(3);
    s.arm(Mode::PhoneLive, StartDelay::Random { min_ms: 800, max_ms: 1200 }, vec![], None);
    s.settle("running");
    // No Stop command: the engine stops on its own.
    for _ in 0..600 {
        s.run_ms(10);
        if s.views.last().is_some_and(|v| v.phase != "running") {
            break;
        }
    }
    let v = s.settle("saved");
    assert_eq!(v.outcome.as_deref(), Some("complete"));
    let r = s.eng.load_review(v.run_id.unwrap()).unwrap();
    let accepted: Vec<i64> = r.events.iter().filter(|e| e.state == "accepted").map(|e| e.timeline_ns / 1_000_000).collect();
    assert_eq!(accepted.len(), 4, "the shot inside the grace counts; the late one is after the stop: {accepted:?}");
    assert!(accepted.iter().all(|t| *t < 2_500));
}

/// Plan phase 2.5, doc 09 capture-duration gate ("continuous 30-minute active-session test
/// without silent discontinuity"), as a desktop simulation through the real engine: data
/// plane, DSP worker, run machine, batched storage. A shot every 10 s must all be accepted,
/// every pushed frame processed, no queue overflow, no interruption, and the run must save
/// complete. Returns a one-line summary for the report.
fn endurance(minutes: i64) -> String {
    let started = std::time::Instant::now();
    let mut s = Sim::new(&format!("endurance-{minutes}"));
    s.record_views = false;
    let shots = (minutes * 6) as usize;
    s.shot_offsets_ms = (0..shots).map(|i| 2_000.0 + 10_000.0 * i as f64).collect();
    s.arm(Mode::PhoneLive, StartDelay::Instant, vec![], None);
    s.record_views = true;
    s.settle("running");
    s.record_views = false;
    let mut durable = 0;
    for m in 0..minutes {
        let v = s.run_ms(60_000);
        assert_eq!(v.phase, "running", "minute {m}: {:?} {:?}", v.interrupt_reason, v.quality_labels);
        assert!(v.interrupt_reason.is_none());
        assert!(v.durable_seq > durable, "minute {m}: observations keep being saved during the run");
        durable = v.durable_seq;
    }
    let d = s.eng.capture_diagnostics().expect("capture open");
    assert_eq!(d.queue_overflows, 0);
    assert!(d.processed_frames >= s.frame - BLOCK as i64, "processed {} of {} frames", d.processed_frames, s.frame);
    s.eng.stop("stop".into(), s.now()).unwrap();
    s.record_views = true;
    let v = s.settle("saved");
    assert_eq!(v.outcome.as_deref(), Some("complete"));
    assert_eq!(v.accepted_count as usize, shots, "every shot accepted, none invented");
    let run = s.eng.list_runs(1).unwrap().remove(0);
    assert_eq!(s.eng.load_review(run.run_id).unwrap().count, shots as u32);
    // The journal and its write-ahead log together.
    let path = db_path_existing(&format!("endurance-{minutes}"));
    let db: u64 = ["", "-wal"].iter().map(|x| std::fs::metadata(format!("{path}{x}")).map(|m| m.len()).unwrap_or(0)).sum();
    format!(
        "{minutes} simulated min: {shots}/{shots} shots, {} blocks, DSP p50 {} us p99 {} us max {} us, queue high water {}/{}, \
         0 overflows, timestamps {}, journal {} KB, wall {:.1} s",
        d.blocks,
        d.dsp_p50_ns / 1000,
        d.dsp_p99_ns / 1000,
        d.dsp_max_ns / 1000,
        d.queue_high_water_blocks,
        d.queue_capacity_blocks,
        d.timestamp_quality,
        db / 1024,
        started.elapsed().as_secs_f64()
    )
}

#[test]
fn capture_runs_continuously_for_two_simulated_minutes() {
    eprintln!("{}", endurance(2));
}

#[test]
#[ignore = "30 simulated minutes: cargo test --release -p squib-mobile --test engine_flows -- --ignored"]
fn capture_runs_continuously_for_thirty_simulated_minutes() {
    let minutes = std::env::var("SQUIB_ENDURANCE_MINUTES").ok().and_then(|m| m.parse().ok()).unwrap_or(30);
    eprintln!("{}", endurance(minutes));
}

#[test]
fn compatibility_report_summarises_live_routes_without_identifiers() {
    let mut s = Sim::new("compat");
    let input = |comment: &str| CompatInput {
        device: "Google Pixel 8".into(),
        android_version: "15 (API 35)".into(),
        app_version: "0.1+1".into(),
        placement: "On a bench or table".into(),
        environment: "Outdoor open range".into(),
        verdict: "Works with corrections".into(),
        comment: comment.into(),
        today_utc_ms: 1_791_694_800_000,
    };
    let empty = s.eng.compatibility_report(input(""));
    assert_eq!(empty.live_runs, 0);
    assert!(empty.text.contains("No live runs yet"));

    s.shot_offsets_ms = vec![700.0, 1000.0, 1350.0];
    s.arm(Mode::PhoneLive, StartDelay::Fixed { ms: 900 }, vec![], Some(3));
    s.settle("running");
    s.run_ms(2000);
    s.eng.stop("stop".into(), s.now()).unwrap();
    s.settle("saved");
    let r = s.eng.compatibility_report(input("Bay 4, \u{7}loud neighbours"));
    assert_eq!((r.live_runs, r.routes), (1, 1));
    let t = &r.text;
    assert!(t.contains("Date: 2026-10-11"), "{t}");
    assert!(t.contains("Device: Google Pixel 8") && t.contains("Placement: On a bench or table"));
    assert!(t.contains("Input builtin_mic"), "{t}");
    assert!(t.contains("Live runs 1 | complete 1 | start cue heard 1 | reviewed with edits 0 | accepted shots 3"), "{t}");
    assert!(t.contains("Clock: [verified_sample_clock x1]"), "{t}");
    assert!(t.contains("Sensitivity setup: none saved"), "{t}");
    assert!(t.contains("Comment: Bay 4, loud neighbours"), "control characters are dropped: {t}");
    // No identifiers or times beyond the date.
    let run = s.eng.list_runs(1).unwrap().remove(0);
    assert!(!t.contains(&run.run_id) && !t.contains(&run.session_id), "{t}");
    assert!(!t.contains("T0") && !t.contains(":00Z"), "no timestamps: {t}");
    let long = s.eng.compatibility_report(input(&"x".repeat(2_000)));
    assert!(long.text.lines().find(|l| l.starts_with("Comment: ")).unwrap().len() <= "Comment: ".len() + 500);
}

/// Security review 5.1: a caller-provided attachment path is checked before any file is
/// read or hashed, so `../` cannot reach files outside the attachment root even briefly.
#[test]
fn attachment_paths_are_validated_before_the_file_is_touched() {
    let s = Sim::new("paths");
    let base = std::env::temp_dir().join(format!("squib-paths-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let root = base.join("attachments");
    std::fs::create_dir_all(&root).unwrap();
    // A readable file outside the root: before the fix it was read (or hashed) first and only
    // the database insert refused the path.
    std::fs::write(base.join("secret.txt"), b"outside").unwrap();
    let run = s.eng.add_manual_run("Club timer".into(), 10, "1.0".into(), None, None, 1_700_000_000_000, 0, "t".into()).unwrap();
    let root_s = root.to_string_lossy().to_string();
    for bad in ["../secret.txt", "/etc/hostname", "photos/../../secret.txt", "a\\b.jpg"] {
        match s.eng.register_attachment(run.clone(), root_s.clone(), bad.into(), "image/jpeg".into(), true, 1) {
            Err(SquibError::Invalid(m)) => assert!(m.contains("app-relative"), "{bad}: {m}"),
            other => panic!("{bad}: {other:?}"),
        }
        match s.eng.register_video(run.clone(), root_s.clone(), bad.into(), clip_meta(1, 1), 1) {
            Err(SquibError::Invalid(m)) => assert!(m.contains("app-relative"), "{bad}: {m}"),
            other => panic!("{bad}: {other:?}"),
        }
    }
    assert!(s.eng.run_attachments(run).is_empty());
}

// ---- Plan 5.3: panic containment and concurrent calls ---------------------------------

fn complete_run(s: &mut Sim, shots: Vec<f64>) -> EngineView {
    s.utc += 60_000;
    s.shot_offsets_ms = shots;
    s.arm(Mode::PhoneLive, StartDelay::Instant, vec![], None);
    s.settle("running");
    s.run_ms(1500);
    s.eng.stop(format!("stop-{}", s.utc), s.now()).unwrap();
    s.settle("saved")
}

#[test]
fn an_analysis_thread_panic_interrupts_the_run_and_the_next_run_works() {
    let mut s = Sim::new("dsp-panic");
    s.shot_offsets_ms = vec![500.0, 900.0];
    s.arm(Mode::PhoneLive, StartDelay::Instant, vec![], None);
    s.settle("running");
    s.eng.debug_inject_panic("dsp".into());
    s.dsp_dead = true;
    let v = s.settle("saved");
    assert_eq!(v.outcome.as_deref(), Some("interrupted"), "never shown as a clean run");
    assert_eq!(v.interrupt_reason.as_deref(), Some("Audio analysis stopped"));
    let r = s.eng.load_review(v.run_id.unwrap()).unwrap();
    assert!(r.quality.iter().any(|q| q.label == "Audio analysis stopped" && q.severity == "integrity"));
    s.dsp_dead = false;
    // Shots the simulator queued for the interrupted run never played; drop them.
    s.inject.clear();
    let v = complete_run(&mut s, vec![400.0, 800.0]);
    assert_eq!((v.outcome.as_deref(), v.accepted_count), (Some("complete"), 2), "a fresh worker for the next run");
}

#[test]
fn a_panic_in_an_engine_call_leaves_the_engine_usable() {
    let mut s = Sim::new("call-panic");
    let eng = s.eng.clone();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| eng.debug_inject_panic("call".into())));
    assert!(r.is_err(), "the call panicked (UniFFI turns this into an exception on the platform side)");
    // The engine lock was poisoned by that panic; later calls recover it.
    let v = complete_run(&mut s, vec![400.0, 800.0, 1200.0]);
    assert_eq!((v.outcome.as_deref(), v.accepted_count), (Some("complete"), 3));
}

#[test]
fn a_panic_on_the_writer_thread_does_not_stop_later_saves() {
    let mut s = Sim::new("store-panic");
    let t = std::time::Instant::now();
    s.eng.debug_inject_panic("store".into());
    assert!(t.elapsed() < Duration::from_secs(2), "reported at once");
    let v = complete_run(&mut s, vec![400.0, 800.0]);
    assert_eq!((v.outcome.as_deref(), v.persist.as_deref()), (Some("complete"), Some("saved")));
    let drills = s.eng.list_drills().len();
    let mut d = s.eng.list_drills().remove(0).input;
    d.title = "After the panic".into();
    s.eng.create_drill(d, s.utc).unwrap();
    assert_eq!(s.eng.list_drills().len(), drills + 1);
}

/// Several platform threads call the engine at once (UI reads, settings and drill writes,
/// conditions) while a live run is captured and saved. Nothing deadlocks, every write
/// lands exactly once, and the runs are timed as if alone.
#[test]
fn concurrent_calls_from_several_threads_during_live_runs() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let mut s = Sim::new("threads");
    let stop = Arc::new(AtomicBool::new(false));
    let created = Arc::new(AtomicUsize::new(0));
    let before = s.eng.list_drills().len();
    let template = s.eng.list_drills().remove(0).input;
    let (done_tx, done_rx) = std::sync::mpsc::channel::<usize>();
    for t in 0..6 {
        let (eng, stop, created, done, mut d) = (s.eng.clone(), stop.clone(), created.clone(), done_tx.clone(), template.clone());
        std::thread::spawn(move || {
            let mut calls = 0;
            while !stop.load(Ordering::Relaxed) {
                match (t + calls) % 6 {
                    0 => drop(eng.list_drills()),
                    1 => drop(eng.list_runs(20)),
                    2 => drop(eng.conditions_view(1_700_000_000_000)),
                    3 => {
                        d.title = format!("Thread {t} drill {calls}");
                        if eng.create_drill(d.clone(), 1_700_000_000_000).is_ok() {
                            created.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    4 => {
                        let _ = eng.add_saved_place(format!("Place {t}-{calls}"), 40.0, -105.0, 1_700_000_000_000);
                    }
                    _ => drop(eng.list_sessions(20)),
                }
                calls += 1;
            }
            let _ = done.send(calls);
        });
    }
    for shots in [vec![400.0, 800.0], vec![300.0, 650.0, 1000.0], vec![500.0]] {
        let n = shots.len() as u32;
        let v = complete_run(&mut s, shots);
        assert_eq!((v.outcome.as_deref(), v.accepted_count), (Some("complete"), n), "{v:?}");
    }
    stop.store(true, Ordering::Relaxed);
    let mut total = 0;
    for _ in 0..6 {
        total += done_rx.recv_timeout(Duration::from_secs(30)).expect("a thread is stuck: deadlock");
    }
    assert!(total > 60, "threads made progress alongside the runs ({total} calls)");
    assert_eq!(s.eng.list_drills().len(), before + created.load(Ordering::Relaxed), "each write landed once");
    assert_eq!(s.eng.list_runs(20).unwrap().len(), 3);
}
