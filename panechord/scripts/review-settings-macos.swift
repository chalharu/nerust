import AppKit
import ApplicationServices
import CryptoKit
import ImageIO
import Vision

enum Failure: Error { case message(String) }
func require(_ condition: Bool, _ message: String) throws {
    if !condition { throw Failure.message(message) }
}
func normalized(_ text: String) -> String {
    text.lowercased().filter { $0.isLetter || $0.isNumber }
}
func wait(_ predicate: () throws -> Bool) throws {
    let deadline = Date().addingTimeInterval(15)
    while Date() < deadline {
        if try predicate() { return }
        RunLoop.current.run(until: Date().addingTimeInterval(0.1))
    }
    throw Failure.message("UI condition timed out")
}
func execute(_ binary: String, _ arguments: [String]) throws {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: binary)
    process.arguments = arguments
    try process.run()
    process.waitUntilExit()
    try require(process.terminationStatus == 0, "command failed: \(binary)")
}
struct Window {
    let id: UInt32
    let bounds: CGRect
}
struct Capture {
    let window: Window
    let observations: [VNRecognizedTextObservation]
    let record: [String: Any]
    func location(_ caption: String, bottommost: Bool = false) throws -> CGPoint {
        let candidates = bottommost ? observations.sorted { $0.boundingBox.midY < $1.boundingBox.midY } : observations
        for observation in candidates {
            guard let text = observation.topCandidates(1).first,
                  let range = text.string.range(of: caption, options: .caseInsensitive),
                  let box = try text.boundingBox(for: range)?.boundingBox else { continue }
            return CGPoint(x: window.bounds.minX + box.midX * window.bounds.width,
                           y: window.bounds.minY + (1 - box.midY) * window.bounds.height)
        }
        throw Failure.message("visible caption missing: \(caption)")
    }
}

func review() throws {
    try require(CommandLine.arguments.count == 4, "usage: swift review-settings-macos.swift BINARY OUTPUT BACKEND")
    try require(AXIsProcessTrusted() && CGPreflightScreenCaptureAccess() && CGPreflightPostEventAccess(),
                "Accessibility, Screen Recording and event-post permission required")
    let output = URL(fileURLWithPath: CommandLine.arguments[2], isDirectory: true)
    let backend = CommandLine.arguments[3]
    let files = FileManager.default
    try files.createDirectory(at: output, withIntermediateDirectories: true)
    let sandbox = files.temporaryDirectory.appendingPathComponent("panechord-rpa-\(UUID().uuidString)")
    try files.createDirectory(at: sandbox, withIntermediateDirectories: true)
    defer { try? files.removeItem(at: sandbox) }
    let config = sandbox.appendingPathComponent("settings.json")
    let log = output.appendingPathComponent("application.log")
    files.createFile(atPath: log.path, contents: nil)
    let handle = try FileHandle(forWritingTo: log)
    defer { try? handle.close() }
    let process = Process()
    process.executableURL = URL(fileURLWithPath: CommandLine.arguments[1])
    process.arguments = ["--config", config.path]
    process.standardOutput = handle
    process.standardError = handle
    try process.run()
    defer {
        if process.isRunning { process.terminate(); process.waitUntilExit() }
    }
    let application = AXUIElementCreateApplication(process.processIdentifier)
    let source = CGEventSource(stateID: .combinedSessionState)
    var records = [[String: Any]]()

    func window() -> Window? {
        guard let infos = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] else { return nil }
        for info in infos {
            guard (info[kCGWindowOwnerPID as String] as? Int32) == process.processIdentifier,
                  (info[kCGWindowLayer as String] as? Int) == 0,
                  let id = info[kCGWindowNumber as String] as? UInt32,
                  let data = info[kCGWindowBounds as String] as? NSDictionary,
                  let bounds = CGRect(dictionaryRepresentation: data) else { continue }
            return Window(id: id, bounds: bounds)
        }
        return nil
    }
    func capture(_ name: String, _ expected: [String], _ forbidden: [String] = []) throws -> Capture {
        let destination = output.appendingPathComponent("\(name).png")
        var result: Capture?
        try wait {
            try require(process.isRunning, "application exited before \(name)")
            guard let current = window() else { return false }
            try execute("/usr/sbin/screencapture", ["-x", "-o", "-l", String(current.id), destination.path])
            guard let imageSource = CGImageSourceCreateWithURL(destination as CFURL, nil),
                  let image = CGImageSourceCreateImageAtIndex(imageSource, 0, nil),
                  let pixels = image.dataProvider?.data else { return false }
            try require(image.width > 100 && image.height > 100 && Set(pixels as Data).count > 8, "blank capture")
            let request = VNRecognizeTextRequest()
            request.recognitionLevel = .accurate
            request.recognitionLanguages = ["en-US"]
            try VNImageRequestHandler(cgImage: image).perform([request])
            let observations = request.results ?? []
            let text = observations.compactMap { $0.topCandidates(1).first?.string }.joined(separator: "\n")
            if !expected.allSatisfy({ normalized(text).contains(normalized($0)) }) { return false }
            if forbidden.contains(where: { normalized(text).contains(normalized($0)) }) { return false }
            let hash = SHA256.hash(data: try Data(contentsOf: destination)).map { String(format: "%02x", $0) }.joined()
            result = Capture(window: current, observations: observations, record: [
                "state": name, "image": destination.lastPathComponent, "sha256": hash,
                "width": image.width, "height": image.height, "ocr": text,
                "window": ["x": current.bounds.minX, "y": current.bounds.minY,
                           "width": current.bounds.width, "height": current.bounds.height]
            ])
            return true
        }
        let captured = result!
        records.append(captured.record)
        print("RPA MACOS \(backend) CAPTURE \(name)")
        return captured
    }
    func resize(_ width: Double, _ height: Double) throws {
        var windows: CFTypeRef?
        try require(AXUIElementCopyAttributeValue(application, kAXWindowsAttribute as CFString, &windows) == .success, "AX windows unavailable")
        let target = (windows as! [AXUIElement])[0]
        var size = CGSize(width: width, height: height)
        let value = AXValueCreate(.cgSize, &size)!
        try require(AXUIElementSetAttributeValue(target, kAXSizeAttribute as CFString, value) == .success, "window resize failed")
        try wait {
            guard let current = window() else { return false }
            return abs(current.bounds.width - width) < 2 && abs(current.bounds.height - height) < 2
        }
    }
    func click(_ location: CGPoint) {
        for type in [CGEventType.mouseMoved, .leftMouseDown, .leftMouseUp] {
            let event = CGEvent(mouseEventSource: source, mouseType: type, mouseCursorPosition: location, mouseButton: .left)!
            event.flags = []
            event.post(tap: .cghidEventTap)
        }
    }
    func button(_ caption: String, _ captured: Capture) throws -> CGPoint {
        func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
            var value: CFTypeRef?
            return AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success ? value : nil
        }
        func find(_ element: AXUIElement, _ depth: Int) -> AXUIElement? {
            if depth > 40 { return nil }
            if attribute(element, kAXRoleAttribute) as? String == kAXButtonRole,
               let title = attribute(element, kAXTitleAttribute) as? String,
               normalized(title) == normalized(caption) { return element }
            for child in attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? [] {
                if let found = find(child, depth + 1) { return found }
            }
            return nil
        }
        if let element = find(application, 0),
           let position = attribute(element, kAXPositionAttribute),
           let dimensions = attribute(element, kAXSizeAttribute) {
            var point = CGPoint.zero
            var size = CGSize.zero
            try require(AXValueGetValue(position as! AXValue, .cgPoint, &point)
                        && AXValueGetValue(dimensions as! AXValue, .cgSize, &size), "AX button geometry unavailable")
            point.x += size.width / 2
            point.y += size.height / 2
            try require(captured.window.bounds.contains(point), "AX button outside captured window")
            print("RPA AX BUTTON \(caption) \(point)")
            return point
        }
        return try captured.location(caption, bottommost: true)
    }
    func command(_ code: CGKeyCode) {
        for pressed in [true, false] {
            let modifier = CGEvent(keyboardEventSource: source, virtualKey: 55, keyDown: pressed)!
            modifier.type = .flagsChanged
            modifier.flags = pressed ? .maskCommand : []
            if pressed {
                modifier.post(tap: .cghidEventTap)
                for down in [true, false] {
                    let key = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: down)!
                    key.flags = .maskCommand
                    key.post(tap: .cghidEventTap)
                }
            } else { modifier.post(tap: .cghidEventTap) }
        }
    }
    func edit(_ capture: Capture, _ caption: String, _ text: String) throws {
        var point = try capture.location(caption)
        point.x = capture.window.bounds.minX + 90
        point.y += 26
        click(point)
        command(0)
        let characters = Array(text.utf16)
        for down in [true, false] {
            let event = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: down)!
            event.flags = []
            characters.withUnsafeBufferPointer { event.keyboardSetUnicodeString(stringLength: $0.count, unicodeString: $0.baseAddress!) }
            event.post(tap: .cghidEventTap)
        }
    }

    try wait {
        try require(process.isRunning, "application exited during launch; see application.log")
        return window() != nil
    }
    NSRunningApplication(processIdentifier: process.processIdentifier)?.activate(options: [])
    try resize(660, 520)
    _ = try capture("initial-660", ["Player preferences", "Up to date"])
    try require(!files.fileExists(atPath: config.path), "startup wrote settings")
    try resize(960, 520)
    _ = try capture("initial-960", ["Player preferences", "Up to date"])
    try resize(backend == "appkit" ? 640 : 440, backend == "appkit" ? 420 : 360)
    _ = try capture("initial-compact", ["Player preferences"])
    try resize(660, 520)
    let ready = try capture("ready", ["Up to date"])
    try edit(ready, "Volume", "abc")
    let invalid = try capture("invalid", ["Volume must be an integer"])
    command(1)
    _ = try capture("invalid-save", ["Volume must be an integer"])
    try require(!files.fileExists(atPath: config.path), "invalid Save wrote settings")
    try edit(invalid, "Volume", "42")
    let valid = try capture("valid-volume", ["Unsaved changes"])
    try edit(valid, "Player name", "PanechordRpa")
    _ = try capture("dirty", ["PanechordRpa", "Unsaved changes"])
    command(12)
    let confirmation = try capture("confirm", ["Keep editing", "discard or keep editing"])
    try require(!files.fileExists(atPath: config.path), "Quit bypassed dirty confirmation")
    click(try button("Keep editing", confirmation))
    _ = try capture("keep-editing", ["PanechordRpa", "Unsaved changes"], ["discard or keep editing"])
    command(1)
    _ = try capture("saved", ["Up to date"])
    try wait { files.fileExists(atPath: config.path) }
    let saved = try Data(contentsOf: config)
    let settings = try JSONSerialization.jsonObject(with: saved) as! [String: Any]
    try require(settings["name"] as? String == "PanechordRpa" && settings["volume"] as? Int == 42, "saved values differ from physical edits")
    try saved.write(to: output.appendingPathComponent("saved-settings.json"))
    command(12)
    try wait { !process.isRunning }
    try require(process.terminationStatus == 0, "clean Quit failed")
    let report: [String: Any] = ["backend": backend, "os": ProcessInfo.processInfo.operatingSystemVersionString,
        "input": "external Quartz CGEvent", "capture": "screencapture owned window + Vision OCR", "captures": records,
        "checks": ["no startup write", "invalid save rejected", "dirty Quit veto", "Keep editing physical click", "Cmd+S persisted exact values", "clean Cmd+Q exit"]]
    try JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys]).write(to: output.appendingPathComponent("evidence.json"))
    print("RPA MACOS \(backend) PASS captures=\(records.count) invalid-save quit-veto keep-editing save-json clean-quit")
}

do { try review() }
catch { fputs("RPA MACOS FAIL: \(error)\n", stderr); exit(1) }