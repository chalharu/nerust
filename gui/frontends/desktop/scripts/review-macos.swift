import AppKit
import ApplicationServices
import CryptoKit
import ImageIO
import Vision

enum Failure: Error { case message(String) }
func require(_ condition: Bool, _ message: String) throws {
    if !condition { throw Failure.message(message) }
}
func wait(_ predicate: () throws -> Bool) throws {
    let deadline = Date().addingTimeInterval(20)
    while Date() < deadline {
        if try predicate() { return }
        RunLoop.current.run(until: Date().addingTimeInterval(0.1))
    }
    throw Failure.message("UI condition timed out")
}
func hash(_ data: Data) -> String {
    SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
}

func review() throws {
    try require((4...5).contains(CommandLine.arguments.count), "usage: swift review-macos.swift BINARY ROM OUTPUT [--view-only|--empty-only|--legacy-counter]")
    let mode = CommandLine.arguments.count == 5 ? CommandLine.arguments[4] : "--view-only"
    try require(["--view-only", "--empty-only", "--legacy-counter"].contains(mode), "unknown review mode")
    let viewOnly = mode != "--legacy-counter"
    let emptyOnly = mode == "--empty-only"
    try require(AXIsProcessTrusted() && CGPreflightScreenCaptureAccess() && CGPreflightPostEventAccess(), "AX, screen-capture and event-post permissions required")
    let files = FileManager.default
    let original = URL(fileURLWithPath: CommandLine.arguments[2])
    let output = URL(fileURLWithPath: CommandLine.arguments[3], isDirectory: true)
    try files.createDirectory(at: output, withIntermediateDirectories: true)
    let reportURL = output.appendingPathComponent("evidence.json")
    if files.fileExists(atPath: reportURL.path) { try files.removeItem(at: reportURL) }
    let sandbox = files.temporaryDirectory.appendingPathComponent("nerust-desktop-rpa-\(UUID().uuidString)")
    try files.createDirectory(at: sandbox, withIntermediateDirectories: true)
    defer { try? files.removeItem(at: sandbox) }
    let rom = sandbox.appendingPathComponent(original.lastPathComponent)
    try files.copyItem(at: original, to: rom)
    if viewOnly {
        let config = sandbox.appendingPathComponent("settings/config", isDirectory: true)
        try files.createDirectory(at: config, withIntermediateDirectories: true)
        let fixture: [String: Any] = ["shared": [String: Any](), "app_state": [String: Any](),
            "local": ["audio": ["muted": true], "notifications": ["status_enabled": true, "info_duration_ms": 10000]]]
        try JSONSerialization.data(withJSONObject: fixture, options: [.sortedKeys]).write(to: config.appendingPathComponent("settings.yaml"))
    }
    let log = output.appendingPathComponent("application.log")
    files.createFile(atPath: log.path, contents: nil)
    let handle = try FileHandle(forWritingTo: log)
    defer { try? handle.close() }
    let process = Process()
    process.executableURL = URL(fileURLWithPath: CommandLine.arguments[1])
    process.arguments = emptyOnly ? [] : [rom.path]
    process.environment = ProcessInfo.processInfo.environment.merging(["NERUST_SETTINGS_ROOT": sandbox.appendingPathComponent("settings").path]) { _, next in next }
    process.standardOutput = handle
    process.standardError = handle
    try process.run()
    defer { if process.isRunning { process.terminate(); process.waitUntilExit() } }
    let application = AXUIElementCreateApplication(process.processIdentifier)
    let events = CGEventSource(stateID: .combinedSessionState)
    var records = [[String: Any]]()

    func key(_ code: CGKeyCode, pressed: Bool) {
        let event = CGEvent(keyboardEventSource: events, virtualKey: code, keyDown: pressed)!
        event.flags = []
        event.post(tap: .cghidEventTap)
    }
    func pulse(_ code: CGKeyCode) {
        key(code, pressed: true)
        key(code, pressed: false)
    }
    func window() -> (UInt32, CGRect)? {
        let infos = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
        for info in infos {
            guard (info[kCGWindowOwnerPID as String] as? Int32) == process.processIdentifier,
                  (info[kCGWindowLayer as String] as? Int) == 0,
                  let id = info[kCGWindowNumber as String] as? UInt32,
                  let data = info[kCGWindowBounds as String] as? NSDictionary,
                  let bounds = CGRect(dictionaryRepresentation: data) else { continue }
            return (id, bounds)
        }
        return nil
    }
    func nativeWindow() throws -> AXUIElement {
        var found: AXUIElement?
        try wait {
            try require(process.isRunning, "application exited before AX window became available")
            var value: CFTypeRef?
            guard AXUIElementCopyAttributeValue(application, kAXWindowsAttribute as CFString, &value) == .success,
                  let windows = value as? [AXUIElement] else { return false }
            found = windows.first
            return found != nil
        }
        guard let found else { throw Failure.message("native window missing") }
        return found
    }
    func resize(_ width: Double, _ height: Double) throws {
        var size = CGSize(width: width, height: height)
        try require(AXUIElementSetAttributeValue(try nativeWindow(), kAXSizeAttribute as CFString, AXValueCreate(.cgSize, &size)!) == .success, "resize rejected")
        try wait {
            guard let (_, bounds) = window() else { return false }
            return abs(bounds.width - width) < 2 && abs(bounds.height - height) < 2
        }
    }
    @discardableResult
    func capture(_ name: String, after: Int = 30, paused: Bool? = nil, forbidden: [String] = [], activeGame: Bool = false, blank: Bool = false) throws -> Int {
        let destination = output.appendingPathComponent("\(name).png")
        var counter = 0
        var record = [String: Any]()
        try wait {
            try require(process.isRunning, "application exited; see application.log")
            guard let (id, bounds) = window() else { return false }
            let camera = Process()
            camera.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
            camera.arguments = ["-x", "-o", "-l", String(id), destination.path]
            try camera.run()
            camera.waitUntilExit()
            try require(camera.terminationStatus == 0, "capture failed")
            guard let source = CGImageSourceCreateWithURL(destination as CFURL, nil),
                  let image = CGImageSourceCreateImageAtIndex(source, 0, nil),
                  let data = image.dataProvider?.data else { return false }
            let request = VNRecognizeTextRequest()
            request.recognitionLevel = .accurate
            request.recognitionLanguages = ["en-US"]
            try VNImageRequestHandler(cgImage: image).perform([request])
            let text = (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }.joined(separator: "\n")
            if activeGame {
                let hud = (request.results ?? []).filter { $0.boundingBox.midY > 0.80 }.compactMap { $0.topCandidates(1).first?.string }.joined(separator: " ")
                if hud.range(of: "\\b(400|3[0-9]{2})\\b", options: .regularExpression) == nil { return false }
            }
            if let paused, text.contains("Paused") != paused { return false }
            if forbidden.contains(where: { text.localizedCaseInsensitiveContains($0) }) { return false }
            if !viewOnly {
                let expression = try NSRegularExpression(pattern: "(?i)frame\\s+(\\d+)")
                guard let match = expression.firstMatch(in: text, range: NSRange(text.startIndex..., in: text)),
                  let range = Range(match.range(at: 1), in: text),
                  let observed = Int(text[range]), observed > after else { return false }
                counter = observed
            }
            try require(image.bitsPerPixel == 32, "unexpected capture pixel format")
            let bytes = data as Data
            var colors = Set<UInt32>()
            var black = true
            for row in stride(from: image.height / 5, to: image.height * 4 / 5, by: 3) {
                for column in stride(from: image.width / 10, to: image.width * 9 / 10, by: 3) {
                    let offset = row * image.bytesPerRow + column * 4
                    let color = UInt32(bytes[offset]) << 24 | UInt32(bytes[offset + 1]) << 16 | UInt32(bytes[offset + 2]) << 8 | UInt32(bytes[offset + 3])
                    colors.insert(color)
                    black = black && bytes[offset] <= 2 && bytes[offset + 1] <= 2 && bytes[offset + 2] <= 2
                }
            }
            if blank ? !black : colors.count <= 8 { return false }
            record = ["state": name, "image": destination.lastPathComponent, "sha256": hash(try Data(contentsOf: destination)),
                      "frame_counter": counter, "center_colors": colors.count, "width": image.width, "height": image.height,
                      "ocr": text, "window_width": bounds.width, "window_height": bounds.height]
            if viewOnly { record.removeValue(forKey: "frame_counter") }
            return true
        }
        records.append(record)
        print("DESKTOP RPA MACOS CAPTURE \(name)" + (viewOnly ? "" : " frame=\(counter)"))
        return counter
    }
    try wait { try require(process.isRunning, "application exited during startup"); return window() != nil }
    NSRunningApplication(processIdentifier: process.processIdentifier)?.activate(options: [])
    try wait { NSWorkspace.shared.frontmostApplication?.processIdentifier == process.processIdentifier }
    try resize(660, 520)
    if emptyOnly {
        _ = try capture("empty", blank: true)
    } else if viewOnly {
        _ = try capture("running", forbidden: ["Frame "])
        pulse(49)
        _ = try capture("paused", paused: true, forbidden: ["Frame "])
    } else {
    let first = try capture("running-660")
    _ = try capture("running-progress", after: first + 10)
    key(9, pressed: true)
    let playing = try capture("playing", after: first + 60, activeGame: true)
    key(9, pressed: false)
    pulse(49)
    let stopped = try capture("paused", after: playing - 1, paused: true)
    try resize(960, 640)
    let resized = try capture("paused-resize", paused: true)
    try require(resized == stopped, "paused resize advanced emulation")
    NSWorkspace.shared.runningApplications.first { $0.bundleIdentifier == "com.apple.finder" }?.activate(options: [])
    try wait { NSWorkspace.shared.frontmostApplication?.processIdentifier != process.processIdentifier }
    NSRunningApplication(processIdentifier: process.processIdentifier)?.activate(options: [])
    try wait { NSWorkspace.shared.frontmostApplication?.processIdentifier == process.processIdentifier }
    let refocused = try capture("paused-refocus", paused: true)
    try require(refocused == stopped, "focus regain released user pause")
    pulse(49)
    _ = try capture("resumed", after: stopped + 10, paused: false)
    try resize(960, 640)
    _ = try capture("running-960")
    try resize(440, 360)
    _ = try capture("running-compact")
    }
    var button: CFTypeRef?
    try require(AXUIElementCopyAttributeValue(try nativeWindow(), kAXCloseButtonAttribute as CFString, &button) == .success, "native close missing")
    try require(AXUIElementPerformAction(button as! AXUIElement, kAXPressAction as CFString) == .success, "native close failed")
    try wait { !process.isRunning }
    try require(process.terminationStatus == 0, "native close did not exit cleanly")
    let report: [String: Any] = ["os": ProcessInfo.processInfo.operatingSystemVersionString,
        "rom_sha256": hash(try Data(contentsOf: original)), "captures": records,
        "mode": mode,
        "checks": viewOnly ? ["sandboxed ROM and settings", emptyOnly ? "black empty client area" : "nonblank game pixels and Paused OSD", "no diagnostic frame-counter footer", "native close exit zero"] : ["sandboxed ROM and settings", "real frame counter progresses", "nonblank game pixels", "external V (default NES Start binding) starts game with active TIME HUD", "external Space pauses and resumes", "paused resize retains counter", "focus regain retains user pause", "native resize", "native close exit zero"]]
    try JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys]).write(to: output.appendingPathComponent("evidence.json"))
    print("DESKTOP RPA MACOS PASS captures=\(records.count)")
}

do { try review() }
catch { fputs("DESKTOP RPA MACOS FAIL: \(error)\n", stderr); exit(1) }