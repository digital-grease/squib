// Squib on iOS, best effort (plan 7.2, ADR-065).

import SwiftUI

@main
struct SquibApp: App {
    @Environment(\.scenePhase) private var scenePhase
    @StateObject private var model = TimerModel(journalPath: SquibApp.journalPath(), appBuild: SquibApp.build())

    var body: some Scene {
        WindowGroup {
            ContentView(model: model)
        }
        .onChange(of: scenePhase) { _, phase in
            // Timing is not trusted in the background: the core interrupts the run.
            if phase == .background { model.backgrounded() }
        }
    }

    /// The journal in Application Support. UI tests pass `-squibFreshJournal YES` for an
    /// empty one of their own.
    static func journalPath() -> String {
        let fm = FileManager.default
        if UserDefaults.standard.bool(forKey: "squibFreshJournal") {
            let dir = fm.temporaryDirectory.appendingPathComponent("squib-ui-\(UUID().uuidString)")
            try? fm.createDirectory(at: dir, withIntermediateDirectories: true)
            return dir.appendingPathComponent("journal.db").path
        }
        let dir = (try? fm.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true))
            ?? fm.temporaryDirectory
        return dir.appendingPathComponent("journal.db").path
    }

    static func build() -> String {
        let info = Bundle.main.infoDictionary
        let v = info?["CFBundleShortVersionString"] as? String ?? "0"
        let b = info?["CFBundleVersion"] as? String ?? "0"
        return "ios-\(v)+\(b)"
    }
}
