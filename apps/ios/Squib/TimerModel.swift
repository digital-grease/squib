// Screen state for the par timer (plan 7.2, ADR-065). The engine and SquibKit's
// ParTimer live on one serial queue (TimerDriver); the screen only receives views.

import AVFoundation
import Foundation
import SquibKit
import SwiftUI
import UIKit

/// Owns the engine and the adapter loop on its own queue. Every ParTimer call happens
/// on `queue`; results are handed to the main queue.
final class TimerDriver {
    struct Update {
        var view: EngineView
        var runs: [RunListItem]?
        var keepScreenOn: Bool
    }

    private let queue = DispatchQueue(label: "net.digitalgrease.squib.timer", qos: .userInteractive)
    private var timer: ParTimer?
    private var looping = false
    private let onUpdate: (Update) -> Void

    init(onUpdate: @escaping (Update) -> Void) {
        self.onUpdate = onUpdate
    }

    /// Open the journal and the audio; `done` gets an error message, and whether sound works.
    func open(journalPath: String, appBuild: String, done: @escaping (String?, Bool) -> Void) {
        queue.async {
            let clock = HostClock()
            var sound = true
            let output: CueOutput
            do {
                output = try AudioCueOutput(clock: clock)
            } catch {
                output = SilentCueOutput()
                sound = false
            }
            do {
                let engine = try SquibEngine(dbPath: journalPath, nowUtcMs: Self.utcMs())
                let t = ParTimer(engine: engine, clock: clock, cues: output, appBuild: appBuild, random: { Double.random(in: 0 ..< 1) })
                self.timer = t
                self.publish(t, runs: t.recentRuns())
                DispatchQueue.main.async { done(nil, sound) }
            } catch {
                DispatchQueue.main.async { done("The journal could not be opened: \(error)", sound) }
            }
        }
    }

    static func utcMs() -> Int64 { Int64(Date().timeIntervalSince1970 * 1000) }

    private func publish(_ t: ParTimer, runs: [RunListItem]?) {
        let u = Update(view: t.view, runs: runs, keepScreenOn: t.keepScreenOn)
        DispatchQueue.main.async { self.onUpdate(u) }
    }

    func arm(_ s: ParSettings, failed: @escaping (String) -> Void) {
        queue.async {
            guard let t = self.timer else { return }
            do {
                let tz = Int32(TimeZone.current.secondsFromGMT() / 60)
                try t.arm(s, nowUtcMs: Self.utcMs(), tzOffsetMin: tz)
                self.publish(t, runs: nil)
                self.startLoop()
            } catch {
                let m = "\(error)"
                DispatchQueue.main.async { failed(m) }
            }
        }
    }

    func stop() { command { try $0.stop() } }
    func cancel() { command { try $0.cancel() } }

    func interrupted(_ e: LifecycleEvent) {
        queue.async {
            guard let t = self.timer, t.view.active else { return }
            t.interrupted(e)
            self.publish(t, runs: nil)
            self.startLoop()
        }
    }

    private func command(_ f: @escaping (ParTimer) throws -> Void) {
        queue.async {
            guard let t = self.timer else { return }
            try? f(t)
            self.publish(t, runs: nil)
            self.startLoop()
        }
    }

    /// Runs on `queue`: poll at the core's deadlines until the run is over and saved.
    private func startLoop() {
        if looping { return }
        looping = true
        turn()
    }

    private func turn() {
        guard let t = timer else { looping = false; return }
        let (_, wait) = t.step()
        guard let wait else {
            looping = false
            publish(t, runs: t.recentRuns())
            return
        }
        publish(t, runs: nil)
        queue.asyncAfter(deadline: .now() + .nanoseconds(Int(wait))) { self.turn() }
    }
}

@MainActor
final class TimerModel: ObservableObject {
    enum DelayKind: String, CaseIterable, Identifiable {
        case instant = "Instant", fixed = "Fixed", random = "Random"
        var id: String { rawValue }
    }

    @Published var delayKind: DelayKind = .random
    @Published var fixedSeconds = "3"
    @Published var randomMinSeconds = "2"
    @Published var randomMaxSeconds = "4"
    @Published var parsText = ""
    @Published var autoStop = true
    @Published private(set) var view: EngineView?
    @Published private(set) var runs: [RunListItem] = []
    @Published var message: String?
    @Published private(set) var soundWorks = true
    @Published private(set) var ready = false

    private var driver: TimerDriver!

    init(journalPath: String, appBuild: String) {
        driver = TimerDriver { [weak self] u in
            MainActor.assumeIsolated {
                guard let self else { return }
                self.view = u.view
                if let r = u.runs { self.runs = r }
                UIApplication.shared.isIdleTimerDisabled = u.keepScreenOn
            }
        }
        driver.open(journalPath: journalPath, appBuild: appBuild) { [weak self] error, sound in
            MainActor.assumeIsolated {
                self?.message = error
                self?.soundWorks = sound
                self?.ready = error == nil
            }
        }
        // A call or another app taking the audio: the core interrupts the run honestly.
        NotificationCenter.default.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: .main) { [weak self] n in
            let began = (n.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt).flatMap(AVAudioSession.InterruptionType.init) == .began
            if began { MainActor.assumeIsolated { self?.driver.interrupted(.audioFocusLost) } }
        }
    }

    var active: Bool { view?.active ?? false }

    private func ms(_ text: String) -> UInt32? {
        guard let s = Double(text.trimmingCharacters(in: .whitespaces).replacingOccurrences(of: ",", with: ".")), s.isFinite, s >= 0, s <= 3600
        else { return nil }
        return UInt32((s * 1000).rounded())
    }

    /// The settings as chosen, or why they cannot be used.
    func settings() -> Result<ParSettings, SquibKitError> {
        let delay: ParSettings.Delay
        switch delayKind {
        case .instant: delay = .instant
        case .fixed:
            guard let f = ms(fixedSeconds) else { return .failure(.invalid("Enter the start delay in seconds.")) }
            delay = .fixed(ms: f)
        case .random:
            guard let lo = ms(randomMinSeconds), let hi = ms(randomMaxSeconds) else {
                return .failure(.invalid("Enter the shortest and longest start delay in seconds."))
            }
            delay = .random(minMs: lo, maxMs: hi)
        }
        guard let pars = ParSettings.parseSeconds(parsText) else {
            return .failure(.invalid("Par times: seconds separated by commas, for example 2.0, 3.5"))
        }
        let s = ParSettings(delay: delay, parsMs: pars, autoStop: autoStop)
        if let p = s.problem() { return .failure(.invalid(p)) }
        return .success(s)
    }

    func start() {
        message = nil
        switch settings() {
        case let .failure(e): message = e.description
        case let .success(s): driver.arm(s) { [weak self] m in self?.message = m.prefix(1).uppercased() + m.dropFirst() }
        }
    }

    func stop() { driver.stop() }
    func cancel() { driver.cancel() }
    func backgrounded() { driver.interrupted(.foregroundLost) }

    /// The run's state in words.
    var status: String {
        guard let v = view else { return "Opening" }
        switch v.phase {
        case "ready": return "Ready"
        case "preparing", "armed", "awaiting_cue": return "Wait for the start signal"
        case "running": return v.parsTotal > 0 ? "Running, par \(v.parsPlayed) of \(v.parsTotal)" : "Running"
        case "completing": return "Finishing"
        case "save_pending": return "Saving"
        case "saved": return "Saved"
        case "cancelled": return "Cancelled"
        case "interrupted": return "Interrupted: \(v.interruptReason ?? "the run was cut short")"
        case "failed": return "Failed: \(v.error ?? "unknown")"
        default: return v.phase
        }
    }

    static func outcomeWords(_ r: RunListItem) -> String {
        switch r.outcome {
        case "complete": return "Complete"
        case "cancelled": return "Cancelled"
        case "interrupted": return "Interrupted"
        case let o?: return o.prefix(1).uppercased() + o.dropFirst()
        case nil: return r.persist == "saved" ? "Saved" : "Not saved yet"
        }
    }

    static func startWords(_ r: RunListItem) -> String? {
        switch r.startMethod {
        case "scheduled_render": return "start from the cue's scheduled playback"
        case "requested_only": return "start time not confirmed"
        case nil: return nil
        case let m?: return m.replacingOccurrences(of: "_", with: " ")
        }
    }
}
