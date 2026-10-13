// Cue sound and the clock it is timed on (plan 7.2, ADR-065).
//
// The engine runs on the host clock (mach_absolute_time) in nanoseconds, converted with
// mach_timebase_info, the platform's own conversion (doc 04: never a guessed scale).
// AVAudioPlayerNode schedules each cue at a host time a little ahead, so the time its
// first frame plays is known when it is scheduled; the output latency the session
// reports is added for the time it leaves the speaker. That is what `cue_rendered` is
// given. On a real phone it is unmeasured (no device evidence, ADR-065).

import AVFoundation
import Darwin
import SquibKit

final class HostClock: MonotonicClock {
    private let numer: UInt64
    private let denom: UInt64

    init() {
        var tb = mach_timebase_info_data_t()
        mach_timebase_info(&tb)
        numer = UInt64(tb.numer)
        denom = UInt64(tb.denom)
    }

    func nowNs() -> Int64 { ns(fromHost: mach_absolute_time()) }

    /// Split so the multiplication cannot overflow for any uptime.
    func ns(fromHost h: UInt64) -> Int64 {
        Int64((h / denom) * numer + (h % denom) * numer / denom)
    }

    func host(fromNs ns: Int64) -> UInt64 {
        let n = UInt64(max(ns, 0))
        return (n / numer) * denom + (n % numer) * denom / numer
    }
}

enum AudioCueError: Error {
    case format
}

/// Plays the core's cue sounds through the speaker.
final class AudioCueOutput: CueOutput {
    /// The core renders cue sounds at this rate; the mixer converts to the hardware rate.
    let sampleRateHz: UInt32 = 48_000
    /// Scheduling lead: long enough for the render thread to pick the buffer up in time.
    static let leadNs: Int64 = 50_000_000
    private let clock: HostClock
    private let engine = AVAudioEngine()
    private let player = AVAudioPlayerNode()
    private let format: AVAudioFormat
    private let session = AVAudioSession.sharedInstance()

    init(clock: HostClock) throws {
        self.clock = clock
        // Playback: cues sound with the silent switch on, as a range timer must. No
        // microphone: iOS is par-only.
        try session.setCategory(.playback, mode: .default, options: [])
        try session.setActive(true)
        guard let f = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: Double(sampleRateHz), channels: 1, interleaved: false)
        else { throw AudioCueError.format }
        format = f
        engine.attach(player)
        engine.connect(player, to: engine.mainMixerNode, format: format)
        try engine.start()
        player.play()
    }

    var isRunning: Bool { engine.isRunning && player.isPlaying }

    func play(_ pcm: [Int16], sampleRateHz: UInt32) -> Int64? {
        guard isRunning, sampleRateHz == self.sampleRateHz, !pcm.isEmpty,
              let buf = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(pcm.count)),
              let ch = buf.floatChannelData?[0]
        else { return nil }
        buf.frameLength = AVAudioFrameCount(pcm.count)
        for (i, s) in pcm.enumerated() { ch[i] = Float(s) / 32_768 }
        let startNs = clock.nowNs() + Self.leadNs
        player.scheduleBuffer(buf, at: AVAudioTime(hostTime: clock.host(fromNs: startNs)), options: [], completionHandler: nil)
        return startNs + Int64(session.outputLatency * 1e9)
    }
}

/// Used when sound cannot start (for example no audio output). Cues are not given, so
/// the core records each start as unconfirmed rather than trusting a silent cue.
final class SilentCueOutput: CueOutput {
    let sampleRateHz: UInt32 = 48_000
    func play(_ pcm: [Int16], sampleRateHz: UInt32) -> Int64? { nil }
}
