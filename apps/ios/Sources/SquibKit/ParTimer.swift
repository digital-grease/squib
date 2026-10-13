// The par timer on iOS (plan 7.1, ADR-065): the same adapter contract the Android
// controller follows (SquibController.kt), in plain Swift so it runs and is tested on
// any platform. The core decides everything about the run; this layer only carries out
// its effects (play a cue, keep the screen on) and polls it at its deadlines.
//
// Par-only: no microphone. A run asks for capture only in live mode, which iOS does not
// offer yet (it needs a capture spike on a device, doc 04 "iOS capture").

@_exported import SquibCore

/// The clock the engine runs on. On iOS this must be the clock audio host times use
/// (mach_absolute_time, as nanoseconds), so cue render times and `now` agree.
public protocol MonotonicClock: AnyObject {
    func nowNs() -> Int64
}

/// Plays cue sound. `play` starts it without blocking and returns the time, on the
/// engine's clock, at which its first frame will play (or `nil` when it could not be
/// played: the core then times out the start rather than trust a cue never given).
public protocol CueOutput: AnyObject {
    var sampleRateHz: UInt32 { get }
    func play(_ pcm: [Int16], sampleRateHz: UInt32) -> Int64?
}

/// What the shooter chose on the timer screen.
public struct ParSettings: Equatable, Sendable {
    public enum Delay: Equatable, Sendable {
        case instant
        case fixed(ms: UInt32)
        case random(minMs: UInt32, maxMs: UInt32)
    }

    public var delay: Delay
    /// Each par, in milliseconds from the start signal.
    public var parsMs: [UInt32]
    /// Stop on its own about a second after the last par (without pars the run lasts
    /// until Stop).
    public var autoStop: Bool

    public init(delay: Delay = .random(minMs: 2000, maxMs: 4000), parsMs: [UInt32] = [], autoStop: Bool = true) {
        self.delay = delay
        self.parsMs = parsMs
        self.autoStop = autoStop
    }

    /// The longest start delay, as on Android (the core enforces its own maximum too).
    public static let maxDelayMs: UInt32 = 30_000

    /// A reason the settings cannot be used, in words, or `nil`. The core checks the
    /// par times themselves (count, length, order, overlap) and says why it refuses.
    public func problem() -> String? {
        switch delay {
        case .instant: return nil
        case let .fixed(ms): return ms > Self.maxDelayMs ? "The start delay must be at most 30 seconds." : nil
        case let .random(lo, hi):
            if lo > hi { return "The shortest random delay must not be longer than the longest." }
            return hi > Self.maxDelayMs ? "The longest random delay must be at most 30 seconds." : nil
        }
    }

    /// Par times typed as seconds separated by commas or spaces ("1.5, 3"), or `nil`
    /// when any part is not a number.
    public static func parseSeconds(_ text: String) -> [UInt32]? {
        let parts = text.split(whereSeparator: { $0 == "," || $0 == " " || $0 == ";" }).filter { !$0.isEmpty }
        var out: [UInt32] = []
        for p in parts {
            guard let s = Double(p), s.isFinite, s > 0, s <= 600 else { return nil }
            out.append(UInt32((s * 1000).rounded()))
        }
        return out
    }
}

/// Drives one engine for par-only runs. Not thread-safe: call it from one serial queue
/// (the app's timer queue, or a test).
public final class ParTimer {
    public let engine: SquibEngine
    private let clock: MonotonicClock
    private let cues: CueOutput
    private let appBuild: String
    private let random: () -> Double
    private var commandSeq = 0
    public private(set) var view: EngineView
    /// What the screen should do about staying awake, as the core asked.
    public private(set) var keepScreenOn = false
    /// Cues the output could not play during this run.
    public private(set) var failedCues = 0

    public init(engine: SquibEngine, clock: MonotonicClock, cues: CueOutput, appBuild: String, random: @escaping () -> Double) {
        self.engine = engine
        self.clock = clock
        self.cues = cues
        self.appBuild = appBuild
        self.random = random
        view = engine.poll(nowNs: clock.nowNs())
    }

    /// The route a par-only run reports: the phone's speaker, no microphone needed.
    public static func parOnlyRoute(osBuild: String, deviceModel: String, mediaVolume: Float) -> RouteReport {
        RouteReport(
            inputDevice: "none",
            outputDevice: "builtin_speaker",
            audioSource: "none",
            unprocessedSupported: false,
            effects: [],
            osBuild: osBuild,
            deviceModel: deviceModel,
            mediaVolume: mediaVolume,
            micPermission: false,
            cameraRecording: false
        )
    }

    private func nextCommand(_ what: String) -> String {
        commandSeq += 1
        return "ios-\(what)-\(clock.nowNs())-\(commandSeq)"
    }

    /// Start a par-only run. Throws the core's refusal, with its reason.
    @discardableResult
    public func arm(_ s: ParSettings, nowUtcMs: Int64, tzOffsetMin: Int32) throws -> EngineView {
        if let p = s.problem() { throw SquibKitError.invalid(p) }
        let delay: StartDelay
        switch s.delay {
        case .instant: delay = .instant
        case let .fixed(ms): delay = .fixed(ms: ms)
        case let .random(lo, hi): delay = .random(minMs: lo, maxMs: hi)
        }
        let req = ArmRequest(
            mode: .parOnly,
            delay: delay,
            unitRandom: min(max(random(), 0), 0.999_999),
            parsMs: s.parsMs,
            autoStopGraceMs: s.autoStop && !s.parsMs.isEmpty ? 1000 : nil,
            autoStopCount: nil,
            expectedCount: nil,
            thresholdDb: nil,
            calibrationId: nil,
            route: nil,
            nowUtcMs: nowUtcMs,
            tzOffsetMin: tzOffsetMin,
            appBuild: appBuild,
            expectedRateHz: cues.sampleRateHz,
            drillId: nil,
            drillVersion: nil,
            planItemId: nil,
            video: false,
            cueChannel: .sound,
            diagnosticRoot: nil
        )
        failedCues = 0
        return apply(try engine.arm(cmdId: nextCommand("arm"), req: req, nowNs: clock.nowNs()))
    }

    @discardableResult
    public func stop() throws -> EngineView {
        apply(try engine.stop(cmdId: nextCommand("stop"), nowNs: clock.nowNs()))
    }

    @discardableResult
    public func cancel() throws -> EngineView {
        apply(try engine.cancel(cmdId: nextCommand("cancel"), nowNs: clock.nowNs()))
    }

    /// The app lost the foreground or its audio: the core interrupts the run honestly.
    @discardableResult
    public func interrupted(_ event: LifecycleEvent) -> EngineView {
        apply(engine.lifecycle(event: event, nowNs: clock.nowNs()))
    }

    /// One turn of the adapter loop: poll, carry out effects, and say how long to wait
    /// before the next turn (nanoseconds), or `nil` when the run is over and saved (or
    /// there is none), so the loop can stop.
    @discardableResult
    public func step() -> (view: EngineView, waitNs: Int64?) {
        let v = apply(engine.poll(nowNs: clock.nowNs()))
        if isIdle(v) { return (v, nil) }
        let ms: Int64 = v.nextDeadlineNs.map { min(max(($0 - clock.nowNs()) / 1_000_000, 1), 15) } ?? 15
        return (v, ms * 1_000_000)
    }

    /// Nothing more will happen without the shooter: no run, or one that is finished and
    /// whose save has been acknowledged (or has failed and is shown as such).
    public func isIdle(_ v: EngineView) -> Bool {
        !v.active && v.persist != "pending"
            && !["completing", "interrupted", "save_pending", "preparing", "armed", "awaiting_cue", "running"].contains(v.phase)
    }

    public func recentRuns(limit: UInt32 = 20) -> [RunListItem] {
        (try? engine.listRuns(limit: limit)) ?? []
    }

    @discardableResult
    private func apply(_ v: EngineView) -> EngineView {
        view = v
        for e in v.effects {
            switch e {
            case let .playCue(cueId, cue, channel):
                // Par-only on iOS gives sound cues only; any other channel is not given,
                // so it is not reported.
                guard channel == .sound else { continue }
                let pcm = engine.cuePcm(cue: cue, sampleRateHz: cues.sampleRateHz)
                let render = cues.play(pcm, sampleRateHz: cues.sampleRateHz)
                if render == nil { failedCues += 1 }
                engine.cueRendered(cueId: cueId, renderNs: render, nowNs: clock.nowNs())
            case let .keepScreenOn(on):
                keepScreenOn = on
            case .startCapture, .stopCapture:
                // Live timing is not offered on iOS; a par-only run never asks for capture.
                break
            }
        }
        return v
    }
}

public enum SquibKitError: Error, Equatable, CustomStringConvertible {
    case invalid(String)

    public var description: String {
        switch self {
        case let .invalid(m): return m
        }
    }
}
