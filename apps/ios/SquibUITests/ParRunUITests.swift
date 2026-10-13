// The par timer end to end on the iOS simulator (plan 7.2, ADR-065): the app, the real
// core and its journal. The simulator's audio is the Mac's, so this checks the flow and
// the saved record, not cue timing.

import XCTest

final class ParRunUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUp() {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments = ["-squibFreshJournal", "YES"]
        app.launch()
    }

    private func waitFor(_ e: XCUIElement, label: String, timeout: TimeInterval = 20) {
        let p = NSPredicate(format: "label BEGINSWITH %@", label)
        let ex = expectation(for: p, evaluatedWith: e)
        wait(for: [ex], timeout: timeout)
    }

    func testLabelledExperimental() {
        let notice = app.staticTexts["notice"]
        XCTAssertTrue(notice.waitForExistence(timeout: 10))
        XCTAssertTrue(notice.label.contains("Experimental") && notice.label.contains("simulator"))
        XCTAssertTrue(app.staticTexts["No runs yet."].exists)
    }

    func testStopEndsARunAndItIsListed() {
        app.buttons["Instant"].tap()
        let start = app.buttons["start"]
        XCTAssertTrue(start.waitForExistence(timeout: 10))
        waitFor(app.staticTexts["phase"], label: "Ready")
        start.tap()
        waitFor(app.staticTexts["phase"], label: "Running")
        app.buttons["stop"].tap()
        waitFor(app.staticTexts["phase"], label: "Saved")
        let row = app.descendants(matching: .any).matching(identifier: "run").firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        XCTAssertTrue(row.label.contains("Complete"), row.label)
    }

    func testParsStopTheRunOnTheirOwn() {
        app.buttons["Instant"].tap()
        let pars = app.textFields["pars"]
        XCTAssertTrue(pars.waitForExistence(timeout: 10))
        pars.tap()
        pars.typeText("0.5, 1")
        waitFor(app.staticTexts["phase"], label: "Ready")
        app.buttons["start"].tap()
        waitFor(app.staticTexts["phase"], label: "Saved")
        let row = app.descendants(matching: .any).matching(identifier: "run").firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        XCTAssertTrue(row.label.contains("Complete"), row.label)
    }

    func testABadDelayIsRefusedWithAReason() {
        app.buttons["Fixed"].tap()
        let field = app.textFields["Seconds"]
        XCTAssertTrue(field.waitForExistence(timeout: 10))
        // Typed after the default 3: 345 seconds, far over the limit.
        field.tap()
        field.typeText("45")
        waitFor(app.staticTexts["phase"], label: "Ready")
        app.buttons["start"].tap()
        let m = app.staticTexts["message"]
        XCTAssertTrue(m.waitForExistence(timeout: 5))
        XCTAssertTrue(m.label.contains("30 seconds"), m.label)
    }
}
