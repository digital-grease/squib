// SquibKit against the real core (plan 7.1, ADR-065), with a clock the test moves and a
// cue output that only records. Runs on the iOS simulator in CI and against the Linux
// build of the core locally; neither says anything about a phone's audio timing.

import Foundation
import XCTest

@testable import SquibKit

final class FakeClock: MonotonicClock {
    var ns: Int64 = 5_000_000_000_000
    func nowNs() -> Int64 { ns }
}

final class FakeCues: CueOutput {
    let sampleRateHz: UInt32 = 48_000
    /// Output latency the fake reports: the cue's first frame plays this long after `play`.
    var latencyNs: Int64 = 10_000_000
    var works = true
    var played: [(at: Int64, frames: Int)] = []
    let clock: FakeClock
    init(clock: FakeClock) { self.clock = clock }

    func play(_ pcm: [Int16], sampleRateHz: UInt32) -> Int64? {
        guard works else { return nil }
        played.append((clock.ns, pcm.count))
        return clock.ns + latencyNs
    }
}

final class ParTimerTests: XCTestCase {
    private var dir: URL!

    override func setUpWithError() throws {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent("squibkit-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: dir)
    }

    private func make(_ name: String = "j") throws -> (ParTimer, FakeClock, FakeCues) {
        let engine = try SquibEngine(dbPath: dir.appendingPathComponent("\(name).db").path, nowUtcMs: 1_791_413_400_000)
        let clock = FakeClock()
        let cues = FakeCues(clock: clock)
        let timer = ParTimer(engine: engine, clock: clock, cues: cues, appBuild: "test", random: { 0.5 })
        return (timer, clock, cues)
    }

    /// Run the adapter loop until it says the run is over, moving the fake clock by the
    /// wait it asks for. Saving is acknowledged by the core's storage thread in real
    /// time, so each turn also yields briefly.
    @discardableResult
    private func runToIdle(_ t: ParTimer, _ clock: FakeClock, limitS: Double = 30) -> EngineView {
        var v = t.view
        let end = clock.ns + Int64(limitS * 1e9)
        while clock.ns < end {
            let (view, wait) = t.step()
            v = view
            guard let wait else { return v }
            clock.ns += wait
            Thread.sleep(forTimeInterval: 0.000_2)
        }
        XCTFail("the run did not finish: phase \(v.phase), persist \(v.persist ?? "none")")
        return v
    }

    func testParRunPlaysEveryCueStopsItselfAndIsSaved() throws {
        let (t, clock, cues) = try make()
        let armedAt = clock.ns
        let v = try t.arm(ParSettings(delay: .fixed(ms: 1500), parsMs: [1000, 2500], autoStop: true), nowUtcMs: 1_791_413_400_000, tzOffsetMin: -300)
        XCTAssertTrue(v.active)
        XCTAssertTrue(t.keepScreenOn, "the screen stays on during a run")

        let done = runToIdle(t, clock)
        XCTAssertEqual(done.phase, "saved")
        XCTAssertEqual(done.persist, "saved", "saved only after the core's durable acknowledgment")
        XCTAssertEqual(done.outcome, "complete")
        XCTAssertEqual(done.startMethod, "scheduled_render", "the cue's render time is the start")
        XCTAssertEqual([done.parsPlayed, done.parsTotal], [2, 2])
        XCTAssertFalse(t.keepScreenOn)

        // Start, then each par at its offset from the start's render time, within the
        // loop's 15 ms polling bound.
        XCTAssertEqual(cues.played.count, 3)
        let start = cues.played[0].at
        XCTAssertEqual(Double(start - armedAt) / 1e6, 1500, accuracy: 16, "fixed delay")
        let render = start + cues.latencyNs
        XCTAssertEqual(Double(cues.played[1].at + cues.latencyNs - render) / 1e6, 1000, accuracy: 16)
        XCTAssertEqual(Double(cues.played[2].at + cues.latencyNs - render) / 1e6, 2500, accuracy: 16)
        XCTAssertTrue(cues.played.allSatisfy { $0.frames > 0 }, "the core's cue sound was played")

        let runs = t.recentRuns()
        XCTAssertEqual(runs.count, 1)
        XCTAssertEqual(runs[0].mode, "par_only")
        XCTAssertEqual(runs[0].outcome, "complete")
        XCTAssertNil(runs[0].count, "par-only records no shots")
    }

    func testStopEndsARunWithoutPars() throws {
        let (t, clock, cues) = try make()
        try t.arm(ParSettings(delay: .instant, parsMs: [], autoStop: true), nowUtcMs: 1_791_413_400_000, tzOffsetMin: 0)
        for _ in 0..<200 {
            if let wait = t.step().waitNs { clock.ns += wait }
            if t.view.phase == "running" { break }
        }
        XCTAssertEqual(t.view.phase, "running")
        clock.ns += 3_000_000_000
        try t.stop()
        let done = runToIdle(t, clock)
        XCTAssertEqual(done.persist, "saved")
        XCTAssertEqual(cues.played.count, 1, "only the start cue")
    }

    func testACueThatCannotPlayIsNotTrusted() throws {
        let (t, clock, cues) = try make()
        cues.works = false
        try t.arm(ParSettings(delay: .instant, parsMs: [800], autoStop: true), nowUtcMs: 1_791_413_400_000, tzOffsetMin: 0)
        let done = runToIdle(t, clock)
        XCTAssertEqual(done.persist, "saved")
        XCTAssertEqual(done.startMethod, "requested_only", "only the request time is known")
        XCTAssertFalse(done.qualityLabels.isEmpty, "and the run says so")
        XCTAssertGreaterThan(t.failedCues, 0)
    }

    func testCancelAndInterruptionAreHonest() throws {
        let (t, clock, cues) = try make()
        try t.arm(ParSettings(delay: .fixed(ms: 5000), parsMs: [1000], autoStop: true), nowUtcMs: 1_791_413_400_000, tzOffsetMin: 0)
        clock.ns += 500_000_000
        t.step()
        try t.cancel()
        XCTAssertEqual(runToIdle(t, clock).phase, "cancelled")
        XCTAssertTrue(cues.played.isEmpty, "cancelled during the delay: no start was given")
        // The attempt is recorded as cancelled, as on Android (engine_flows cancel_before_cue_records_attempt_only).
        XCTAssertEqual(t.recentRuns().map(\.outcome), ["cancelled"])

        try t.arm(ParSettings(delay: .instant, parsMs: [2000], autoStop: true), nowUtcMs: 1_791_413_400_000, tzOffsetMin: 0)
        for _ in 0..<50 { if let w = t.step().waitNs { clock.ns += w } }
        t.interrupted(.foregroundLost)
        let done = runToIdle(t, clock)
        XCTAssertEqual(done.outcome, "interrupted")
    }

    func testRefusalsCarryAReason() throws {
        let (t, _, _) = try make()
        XCTAssertThrowsError(try t.arm(ParSettings(delay: .fixed(ms: 40_000)), nowUtcMs: 0, tzOffsetMin: 0)) {
            XCTAssertEqual("\($0)", "The start delay must be at most 30 seconds.")
        }
        XCTAssertThrowsError(try t.arm(ParSettings(delay: .random(minMs: 3000, maxMs: 1000)), nowUtcMs: 0, tzOffsetMin: 0))
        // The core's own check, with its reason.
        XCTAssertThrowsError(try t.arm(ParSettings(delay: .instant, parsMs: [2000, 1000]), nowUtcMs: 0, tzOffsetMin: 0)) {
            XCTAssertTrue("\($0)".contains("increasing"), "\($0)")
        }
        XCTAssertFalse(t.view.active)
    }

    func testParTimesAreTypedInSeconds() {
        XCTAssertEqual(ParSettings.parseSeconds("1.5, 3"), [1500, 3000])
        XCTAssertEqual(ParSettings.parseSeconds(" 2 4;6 "), [2000, 4000, 6000])
        XCTAssertEqual(ParSettings.parseSeconds(""), [])
        XCTAssertNil(ParSettings.parseSeconds("1.5, x"))
        XCTAssertNil(ParSettings.parseSeconds("-1"))
        XCTAssertNil(ParSettings.parseSeconds("700"))
    }
}
