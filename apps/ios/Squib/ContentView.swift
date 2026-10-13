// The par timer screen (plan 7.2, ADR-065). Experimental, par-only, simulator-tested.

import SquibKit
import SwiftUI

struct ContentView: View {
    @ObservedObject var model: TimerModel

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Text("Experimental on iOS: a par timer only, tested on the simulator, not on an iPhone. Shot timing is on Android.")
                        .font(.footnote)
                        .accessibilityIdentifier("notice")
                    if !model.soundWorks {
                        Text("Sound could not start, so cues will not play and start times are not confirmed.")
                            .foregroundStyle(.red)
                    }
                }

                Section("Timer") {
                    Text(model.status)
                        .font(.title2.weight(.semibold))
                        .accessibilityIdentifier("phase")
                    if model.active {
                        HStack {
                            Button("Stop") { model.stop() }
                                .buttonStyle(.borderedProminent)
                                .accessibilityIdentifier("stop")
                            Button("Cancel", role: .destructive) { model.cancel() }
                                .buttonStyle(.bordered)
                                .accessibilityIdentifier("cancel")
                        }
                    } else {
                        Button("Start") { model.start() }
                            .buttonStyle(.borderedProminent)
                            .disabled(!model.ready)
                            .accessibilityIdentifier("start")
                    }
                    if let m = model.message {
                        Text(m).foregroundStyle(.red).accessibilityIdentifier("message")
                    }
                }

                Section("Recent runs") {
                    if model.runs.isEmpty {
                        Text("No runs yet.").foregroundStyle(.secondary)
                    }
                    ForEach(model.runs.prefix(5), id: \.runId) { r in
                        RunRow(run: r)
                    }
                }

                Section("Start delay") {
                    Picker("Start delay", selection: $model.delayKind) {
                        ForEach(TimerModel.DelayKind.allCases) { Text($0.rawValue).tag($0) }
                    }
                    .pickerStyle(.segmented)
                    .accessibilityIdentifier("delay")
                    switch model.delayKind {
                    case .instant:
                        Text("The start signal sounds as soon as you press Start.").font(.footnote)
                    case .fixed:
                        TextField("Seconds", text: $model.fixedSeconds)
                            .keyboardType(.decimalPad)
                            .accessibilityLabel("Start delay, seconds")
                    case .random:
                        TextField("Shortest, seconds", text: $model.randomMinSeconds)
                            .keyboardType(.decimalPad)
                            .accessibilityLabel("Shortest start delay, seconds")
                        TextField("Longest, seconds", text: $model.randomMaxSeconds)
                            .keyboardType(.decimalPad)
                            .accessibilityLabel("Longest start delay, seconds")
                    }
                }
                .disabled(model.active)

                Section("Par times") {
                    TextField("Seconds from the start, for example 2, 3.5", text: $model.parsText)
                        .keyboardType(.numbersAndPunctuation)
                        .accessibilityLabel("Par times, seconds from the start, separated by commas")
                        .accessibilityIdentifier("pars")
                    Toggle("Stop a second after the last par", isOn: $model.autoStop)
                        .accessibilityIdentifier("autostop")
                }
                .disabled(model.active)
            }
            .navigationTitle("Squib")
        }
    }
}

struct RunRow: View {
    let run: RunListItem

    var body: some View {
        let date = Date(timeIntervalSince1970: Double(run.createdUtcMs) / 1000)
        let words = [TimerModel.outcomeWords(run), TimerModel.startWords(run)].compactMap { $0 }.joined(separator: ", ")
        VStack(alignment: .leading) {
            Text(words)
            Text(date.formatted(date: .abbreviated, time: .standard)).font(.footnote).foregroundStyle(.secondary)
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("run")
    }
}
