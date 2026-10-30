import AppKit

public typealias Callback = @convention(c) (UnsafeMutableRawPointer?, Int32, UnsafePointer<UInt8>?, Int, UnsafePointer<UInt8>?, Int) -> Void

func string(_ bytes: UnsafePointer<UInt8>?, _ count: Int) -> String {
    guard let bytes = bytes else { return "" }
    return String(decoding: UnsafeBufferPointer(start: bytes, count: count), as: UTF8.self)
}

@MainActor
final class Rows: NSStackView {
    let body = NSStackView()
    let scroller = NSScroller()
    var changed: ((Int) -> Void)?
    var maximum = 0
    var start = 0
    var rowHeight: CGFloat = 32
    var fixedHeight: NSLayoutConstraint?

    init() {
        super.init(frame: .zero)
        orientation = .horizontal
        alignment = .top
        spacing = 2
        body.orientation = .vertical
        body.alignment = .leading
        body.spacing = 0
        addArrangedSubview(body)
        addArrangedSubview(scroller)
        scroller.target = self
        scroller.action = #selector(scrollChanged)
        body.translatesAutoresizingMaskIntoConstraints = false
        scroller.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            scroller.widthAnchor.constraint(equalToConstant: 16),
            scroller.heightAnchor.constraint(equalTo: heightAnchor),
            body.widthAnchor.constraint(equalTo: widthAnchor, constant: -18)
        ])
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }

    @objc func scrollChanged() {
        let next: Int
        switch scroller.hitPart {
        case .incrementLine: next = start + 1
        case .decrementLine: next = start - 1
        case .incrementPage: next = start + 6
        case .decrementPage: next = start - 6
        default: next = Int((scroller.doubleValue * Double(maximum)).rounded())
        }
        changed?(max(0, min(maximum, next)))
    }

    override func scrollWheel(with event: NSEvent) {
        let delta = event.scrollingDeltaY
        if delta != 0 {
            changed?(max(0, min(maximum, start + (delta < 0 ? 1 : -1))))
        }
    }
}

@MainActor
final class DocumentStack: NSStackView {
    override var isFlipped: Bool { true }
}

@MainActor
final class ApplicationDelegate: NSObject, NSApplicationDelegate {
    static let shared = ApplicationDelegate()
    weak var active: Host?
    @objc func quitRequested() { active?.emit("", kind: 3) }
    func retire(_ value: Host) {
        guard active === value else { return }
        active = nil
        let next = NSApp.orderedWindows.compactMap { $0.delegate as? Host }.first {
            !$0.closed && ($0.window.isVisible || $0.window.isMiniaturized)
        }
        if let next = next {
            next.window.makeKeyAndOrderFront(nil)
            next.windowDidBecomeKey(Notification(name: NSWindow.didBecomeKeyNotification))
        } else {
            NSApp.mainMenu = nil
        }
    }
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        quitRequested()
        return .terminateCancel
    }
}

@MainActor
final class Host: NSObject, NSWindowDelegate, NSTextFieldDelegate {
    let window: NSWindow
    let root = DocumentStack()
    var nodes: [String: NSView] = [:]
    var frames: [String: NSStackView] = [:]
    var dimensions: [String: [NSLayoutConstraint]] = [:]
    var contentWidths: [String: NSLayoutConstraint] = [:]
    var menus: [String: NSMenu] = [:]
    var menuItems: [String: NSMenuItem] = [:]
    var focusable: [NSView] = []
    var callback: Callback?
    var context: UnsafeMutableRawPointer?
    var closed = false
    let bar = NSMenu()

    override init() {
        precondition(Thread.isMainThread)
        window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 760, height: 660),
                          styleMask: [.titled, .closable, .miniaturizable, .resizable],
                          backing: .buffered, defer: false)
        super.init()
        window.isReleasedWhenClosed = false
        window.minSize = NSSize(width: 640, height: 420)
        window.delegate = self
        window.autorecalculatesKeyViewLoop = false
        window.center()
        let scroll = NSScrollView()
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        window.contentView = scroll
        root.orientation = .vertical
        root.alignment = .leading
        root.spacing = 0
        scroll.documentView = root
        root.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            root.leadingAnchor.constraint(equalTo: scroll.contentView.leadingAnchor),
            root.topAnchor.constraint(equalTo: scroll.contentView.topAnchor),
            root.widthAnchor.constraint(equalTo: scroll.contentView.widthAnchor)
        ])
        nodes["__root"] = root
        frames["__root"] = root
        NSApp.delegate = ApplicationDelegate.shared
        let app = NSMenuItem()
        let appMenu = NSMenu(title: "Wakui")
        app.submenu = appMenu
        bar.addItem(app)
        let about = NSMenuItem(title: "About Wakui", action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)), keyEquivalent: "")
        appMenu.addItem(about)
        appMenu.addItem(.separator())
        let quit = NSMenuItem(title: "Quit Wakui", action: #selector(ApplicationDelegate.quitRequested), keyEquivalent: "q")
        quit.target = ApplicationDelegate.shared
        appMenu.addItem(quit)
        let edit = NSMenuItem()
        let editMenu = NSMenu(title: "Edit")
        edit.submenu = editMenu
        bar.addItem(edit)
        for (title, action, key) in [("Undo", Selector(("undo:")), "z"),
                                     ("Redo", Selector(("redo:")), "Z"),
                                     ("Cut", #selector(NSText.cut(_:)), "x"),
                                     ("Copy", #selector(NSText.copy(_:)), "c"),
                                     ("Paste", #selector(NSText.paste(_:)), "v"),
                                     ("Select All", #selector(NSText.selectAll(_:)), "a")] {
            editMenu.addItem(NSMenuItem(title: title, action: action, keyEquivalent: key))
        }
    }

    func windowDidBecomeKey(_ notification: Notification) {
        guard !closed else { return }
        ApplicationDelegate.shared.active = self
        NSApp.mainMenu = bar
    }

    func emit(_ id: String, _ value: String = "", kind: Int32 = 0) {
        guard let callback = callback else { return }
        Array(id.utf8).withUnsafeBufferPointer { key in
            Array(value.utf8).withUnsafeBufferPointer { text in
                callback(context, kind, key.baseAddress, key.count, text.baseAddress, text.count)
            }
        }
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        emit("", kind: 1)
        return false
    }
    @objc func activated(_ sender: NSControl) {
        if let id = sender.identifier?.rawValue { emit(id) }
    }
    @objc func menuActivated(_ sender: NSMenuItem) {
        if let id = sender.representedObject as? String { emit(id) }
    }
    func controlTextDidChange(_ notification: Notification) {
        guard let field = notification.object as? NSTextField,
              let id = field.identifier?.rawValue else { return }
        if let editor = field.currentEditor() as? NSTextView, editor.hasMarkedText() { return }
        emit(id, field.stringValue)
    }

    func attach(_ view: NSView, parent: String, id: String) {
        guard let container = (nodes[parent] as? Rows)?.body ?? nodes[parent] as? NSStackView else { return }
        let frame = NSStackView()
        frame.orientation = .vertical
        frame.alignment = .leading
        frame.spacing = 0
        frame.identifier = NSUserInterfaceItemIdentifier("frame/" + id)
        frame.addArrangedSubview(view)
        container.addArrangedSubview(frame)
        frame.translatesAutoresizingMaskIntoConstraints = false
        view.translatesAutoresizingMaskIntoConstraints = false
        let contentWidth = view.widthAnchor.constraint(equalTo: frame.widthAnchor, constant: 0)
        contentWidth.isActive = true
        contentWidths[id] = contentWidth
        if container.orientation == .vertical {
            frame.widthAnchor.constraint(equalTo: container.widthAnchor,
                constant: -(container.edgeInsets.left + container.edgeInsets.right)).isActive = true
        }
        nodes[id] = view
        frames[id] = frame
        view.identifier = NSUserInterfaceItemIdentifier(id)
        view.setAccessibilityIdentifier(id)
        if view is NSButton || (view as? NSTextField)?.isSelectable == true {
            focusable.append(view)
            keyLoop()
        }
    }

    func keyLoop() {
        for (index, view) in focusable.enumerated() {
            view.nextKeyView = focusable[(index + 1) % focusable.count]
        }
        window.initialFirstResponder = focusable.first
    }

    func remove(_ prefix: String) {
        let ids = nodes.keys.filter { $0 == prefix || $0.hasPrefix(prefix + "/") }
        for id in ids.sorted(by: { $0.count > $1.count }) {
            if let frame = frames.removeValue(forKey: id) {
                if let parent = frame.superview as? NSStackView { parent.removeArrangedSubview(frame) }
                frame.removeFromSuperview()
            }
            dimensions.removeValue(forKey: id)
            contentWidths.removeValue(forKey: id)
            if let node = nodes.removeValue(forKey: id) { focusable.removeAll { $0 === node } }
        }
        keyLoop()
    }
}

@MainActor func host(_ pointer: UnsafeMutableRawPointer) -> Host {
    Unmanaged<Host>.fromOpaque(pointer).takeUnretainedValue()
}

@_cdecl("w3_create") @MainActor
public func create(_ callback: Callback?, _ context: UnsafeMutableRawPointer?) -> UnsafeMutableRawPointer {
    let app = NSApplication.shared
    app.setActivationPolicy(.regular)
    let value = Host()
    value.callback = callback
    value.context = context
    app.finishLaunching()
    return Unmanaged.passRetained(value).toOpaque()
}

@_cdecl("w3_destroy") @MainActor
public func destroy(_ pointer: UnsafeMutableRawPointer) {
    let value = host(pointer)
    value.callback = nil
    value.window.delegate = nil
    value.closed = true
    ApplicationDelegate.shared.retire(value)
    value.window.close()
    Unmanaged<Host>.fromOpaque(pointer).release()
}

@_cdecl("w3_present") @MainActor
public func present(_ pointer: UnsafeMutableRawPointer) {
    host(pointer).window.makeKeyAndOrderFront(nil)
    host(pointer).windowDidBecomeKey(Notification(name: NSWindow.didBecomeKeyNotification))
    NSApp.activate(ignoringOtherApps: true)
}

@_cdecl("w3_close") @MainActor
public func close(_ pointer: UnsafeMutableRawPointer) {
    let value = host(pointer)
    value.closed = true
    value.window.orderOut(nil)
    ApplicationDelegate.shared.retire(value)
}

@_cdecl("w3_focused") @MainActor
public func focused(_ pointer: UnsafeMutableRawPointer) -> Int32 {
    NSApp.isActive && host(pointer).window.isKeyWindow ? 1 : 0
}

@_cdecl("w3_render") @MainActor
public func render(_ pointer: UnsafeMutableRawPointer) {
    let value = host(pointer)
    value.window.contentView?.needsDisplay = true
    value.window.displayIfNeeded()
}

@_cdecl("w3_pump") @MainActor
public func pump(_ milliseconds: Int32) {
    autoreleasepool {
        if let event = NSApp.nextEvent(matching: .any,
            until: Date(timeIntervalSinceNow: Double(milliseconds) / 1000),
            inMode: .default, dequeue: true) { NSApp.sendEvent(event) }
        NSApp.updateWindows()
    }
}

@_cdecl("w3_add") @MainActor
public func add(_ pointer: UnsafeMutableRawPointer, _ parentBytes: UnsafePointer<UInt8>?, _ parentCount: Int,
         _ idBytes: UnsafePointer<UInt8>?, _ idCount: Int, _ kind: Int32, _ number: Double) {
    let value = host(pointer)
    let id = string(idBytes, idCount)
    let view: NSView
    switch kind {
    case 0, 1:
        let stack = NSStackView()
        stack.orientation = kind == 1 ? .horizontal : .vertical
        stack.alignment = kind == 1 ? .centerY : .leading
        stack.spacing = number
        view = stack
    case 2:
        let field = NSTextField(wrappingLabelWithString: "")
        field.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        field.setAccessibilityRole(.staticText)
        view = field
    case 3:
        let field = NSTextField(string: "")
        field.delegate = value
        field.isSelectable = true
        field.isEditable = number != 0
        field.setAccessibilityRole(.textField)
        field.usesSingleLineMode = true
        field.isAutomaticTextCompletionEnabled = false
        if let caption = value.nodes[id + "_caption"] as? NSTextField {
            field.setAccessibilityLabel(caption.stringValue)
        }
        field.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        view = field
    case 4:
        let button = NSButton(title: "", target: value, action: #selector(Host.activated(_:)))
        button.bezelStyle = .rounded
        view = button
    default:
        let rows = Rows()
        rows.changed = { [weak value] start in value?.emit(id, String(start), kind: 2) }
        view = rows
    }
    value.attach(view, parent: string(parentBytes, parentCount), id: id)
}

@_cdecl("w3_set") @MainActor
public func set(_ pointer: UnsafeMutableRawPointer, _ idBytes: UnsafePointer<UInt8>?, _ idCount: Int,
         _ textBytes: UnsafePointer<UInt8>?, _ textCount: Int, _ property: Int32) {
    let value = host(pointer)
    let id = string(idBytes, idCount)
    let text = string(textBytes, textCount)
    if property == 1 {
        (value.nodes[id] as? NSControl)?.isEnabled = text == "1"
    } else if property == 2 {
        value.window.title = text
    } else if let field = value.nodes[id] as? NSTextField {
        if let editor = field.currentEditor() as? NSTextView, editor.hasMarkedText() { return }
        if field.stringValue != text { field.stringValue = text }
    } else if let button = value.nodes[id] as? NSButton { button.title = text }
}

@_cdecl("w3_text") @MainActor
public func text(_ pointer: UnsafeMutableRawPointer, _ idBytes: UnsafePointer<UInt8>?, _ idCount: Int,
          _ output: UnsafeMutablePointer<UInt8>?, _ capacity: Int) -> Int {
    let value = host(pointer)
    let id = string(idBytes, idCount)
    let content: String
    if id == "__root" { content = value.window.title }
    else if let field = value.nodes[id] as? NSTextField { content = field.stringValue }
    else if let button = value.nodes[id] as? NSButton { content = button.title }
    else { return -1 }
    let bytes = Array(content.utf8)
    if let output = output, capacity >= bytes.count {
        bytes.withUnsafeBufferPointer { source in
            if let base = source.baseAddress { output.update(from: base, count: source.count) }
        }
    }
    return bytes.count
}

@_cdecl("w3_action") @MainActor
public func action(_ pointer: UnsafeMutableRawPointer, _ idBytes: UnsafePointer<UInt8>?, _ idCount: Int,
            _ textBytes: UnsafePointer<UInt8>?, _ textCount: Int, _ kind: Int32) -> Int32 {
    let value = host(pointer)
    let id = string(idBytes, idCount)
    if kind == 0, let field = value.nodes[id] as? NSTextField, field.isEditable, field.isEnabled {
        field.stringValue = string(textBytes, textCount)
        value.controlTextDidChange(Notification(name: NSControl.textDidChangeNotification, object: field))
        return 1
    }
    if kind == 1, let button = value.nodes[id] as? NSButton, button.isEnabled {
        button.performClick(nil)
        return 1
    }
    if kind == 2, let item = value.menuItems[id], item.isEnabled,
       let menu = item.menu {
        menu.performActionForItem(at: menu.index(of: item))
        return 1
    }
    return 0
}

@_cdecl("w3_style") @MainActor
public func style(_ pointer: UnsafeMutableRawPointer, _ idBytes: UnsafePointer<UInt8>?, _ idCount: Int,
           _ top: Double, _ right: Double, _ bottom: Double, _ left: Double,
           _ width: Double, _ height: Double) {
    let value = host(pointer)
    let id = string(idBytes, idCount)
    guard let frame = value.frames[id] else { return }
    frame.edgeInsets = NSEdgeInsets(top: top, left: left, bottom: bottom, right: right)
    value.contentWidths[id]?.constant = -(left + right)
    if let constraints = value.dimensions[id] { NSLayoutConstraint.deactivate(constraints) }
    var constraints: [NSLayoutConstraint] = []
    if width > 0 { constraints.append(frame.widthAnchor.constraint(equalToConstant: width)) }
    if height > 0 { constraints.append(frame.heightAnchor.constraint(equalToConstant: height)) }
    NSLayoutConstraint.activate(constraints)
    value.dimensions[id] = constraints
}

@_cdecl("w3_remove") @MainActor
public func remove(_ pointer: UnsafeMutableRawPointer, _ idBytes: UnsafePointer<UInt8>?, _ idCount: Int) {
    host(pointer).remove(string(idBytes, idCount))
}

@_cdecl("w3_frame_height") @MainActor
public func frameHeight(_ pointer: UnsafeMutableRawPointer, _ idBytes: UnsafePointer<UInt8>?, _ idCount: Int) -> Double {
    let value = host(pointer)
    value.window.contentView?.layoutSubtreeIfNeeded()
    return value.frames[string(idBytes, idCount)].map { Double($0.frame.height) } ?? -1
}

@_cdecl("w3_rows") @MainActor
public func rows(_ pointer: UnsafeMutableRawPointer, _ idBytes: UnsafePointer<UInt8>?, _ idCount: Int,
          _ total: Int, _ start: Int, _ count: Int, _ height: Double) {
    let value = host(pointer)
    let id = string(idBytes, idCount)
    guard let rows = value.nodes[id] as? Rows else { return }
    rows.maximum = max(0, total - count)
    rows.start = start
    rows.rowHeight = height
    rows.fixedHeight?.isActive = false
    rows.fixedHeight = rows.heightAnchor.constraint(equalToConstant: Double(count) * height)
    rows.fixedHeight?.isActive = true
    rows.scroller.isEnabled = rows.maximum > 0
    rows.scroller.knobProportion = total > 0 ? min(1, Double(count) / Double(total)) : 1
    rows.scroller.doubleValue = rows.maximum > 0 ? Double(start) / Double(rows.maximum) : 0
}

func shortcut(_ value: String) -> (String, NSEvent.ModifierFlags) {
    let parts = value.split(separator: "+").map(String.init)
    var flags: NSEvent.ModifierFlags = []
    for modifier in parts.dropLast() {
        switch modifier {
        case "Cmd": flags.insert(.command)
        case "Ctrl": flags.insert(.control)
        case "Alt": flags.insert(.option)
        case "Shift": flags.insert(.shift)
        default: break
        }
    }
    let raw = parts.last ?? ""
    if raw.hasPrefix("Key") { return (String(raw.dropFirst(3)).lowercased(), flags) }
    if raw.hasPrefix("Digit") { return (String(raw.dropFirst(5)), flags) }
    let keys: [String: String] = ["Enter": "\r", "Tab": "\t", "Space": " ", "Escape": "\u{1b}",
        "Backspace": "\u{8}", "Delete": "\u{f728}", "Insert": "\u{f727}",
        "Home": "\u{f729}", "End": "\u{f72b}", "PageUp": "\u{f72c}", "PageDown": "\u{f72d}",
        "ArrowLeft": "\u{f702}", "ArrowRight": "\u{f703}", "ArrowUp": "\u{f700}", "ArrowDown": "\u{f701}"]
    if let key = keys[raw] { return (key, flags) }
    if raw.hasPrefix("F"), let number = Int(raw.dropFirst()), let scalar = UnicodeScalar(0xf703 + number) {
        return (String(scalar), flags)
    }
    return ("", flags)
}

@_cdecl("w3_menu") @MainActor
public func menu(_ pointer: UnsafeMutableRawPointer, _ idBytes: UnsafePointer<UInt8>?, _ idCount: Int,
          _ jsonBytes: UnsafePointer<UInt8>?, _ jsonCount: Int) {
    let value = host(pointer)
    let id = string(idBytes, idCount)
    guard let data = string(jsonBytes, jsonCount).data(using: .utf8),
          let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
          let title = object["title"] as? String, let items = object["items"] as? [[String: Any]] else { return }
    let submenu: NSMenu
    if let existing = value.menus[id] { submenu = existing }
    else {
        submenu = NSMenu(title: title)
        submenu.autoenablesItems = false
        let parent = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        parent.submenu = submenu
        value.bar.addItem(parent)
        value.menus[id] = submenu
    }
    submenu.title = title
    value.bar.items.first(where: { $0.submenu === submenu })?.title = title
    let old = value.menuItems.keys.filter { $0.hasPrefix(id + "/") }
    for key in old { value.menuItems.removeValue(forKey: key) }
    submenu.removeAllItems()
    for item in items {
        let role = item["role"] as? String ?? "Normal"
        if role == "Separator" { submenu.addItem(.separator()); continue }
        let key = id + "/" + (item["id"] as? String ?? "")
        let native = NSMenuItem(title: item["label"] as? String ?? "", action: #selector(Host.menuActivated(_:)), keyEquivalent: "")
        native.target = value
        native.representedObject = key
        native.isEnabled = item["enabled"] as? Bool ?? true
        let (equivalent, flags) = shortcut(item["shortcut"] as? String ?? "")
        native.keyEquivalent = equivalent
        native.keyEquivalentModifierMask = flags
        let selectors: [String: Selector] = ["Copy": #selector(NSText.copy(_:)), "Cut": #selector(NSText.cut(_:)),
            "Paste": #selector(NSText.paste(_:)), "SelectAll": #selector(NSText.selectAll(_:)),
            "About": #selector(NSApplication.orderFrontStandardAboutPanel(_:))]
        if let selector = selectors[role] { native.action = selector; native.target = nil }
        if role == "Quit" { native.action = #selector(ApplicationDelegate.quitRequested); native.target = ApplicationDelegate.shared }
        submenu.addItem(native)
        value.menuItems[key] = native
    }
}

@_cdecl("w3_input_probe") @MainActor
public func inputProbe(_ pointer: UnsafeMutableRawPointer, _ idBytes: UnsafePointer<UInt8>?, _ idCount: Int) -> Int32 {
    let value = host(pointer)
    let id = string(idBytes, idCount)
    let pasteboard = NSPasteboard.general
    let saved = (pasteboard.pasteboardItems ?? []).map { original -> NSPasteboardItem in
        let copy = NSPasteboardItem()
        for type in original.types {
            if let data = original.data(forType: type) { copy.setData(data, forType: type) }
        }
        return copy
    }
    defer { pasteboard.clearContents(); if !saved.isEmpty { pasteboard.writeObjects(saved) } }
    guard let field = value.nodes[id] as? NSTextField, field.isEditable else { return -1 }
    value.window.makeFirstResponder(field)
    guard let editor = field.currentEditor() as? NSTextView else { return -2 }
    editor.selectAll(nil)
    editor.setMarkedText("kana", selectedRange: NSRange(location: 4, length: 0), replacementRange: NSRange(location: NSNotFound, length: 0))
    guard editor.hasMarkedText() else { return -3 }
    set(pointer, idBytes, idCount, nil, 0, 0)
    guard editor.hasMarkedText(), editor.string == "kana" else { return -4 }
    editor.insertText("\u{65e5}\u{672c}\u{8a9e}", replacementRange: editor.markedRange())
    guard !editor.hasMarkedText() else { return -5 }
    editor.selectAll(nil)
    editor.copy(nil)
    guard NSPasteboard.general.string(forType: .string) == "\u{65e5}\u{672c}\u{8a9e}" else { return -6 }
    editor.cut(nil)
    guard editor.string.isEmpty else { return -7 }
    editor.paste(nil)
    guard editor.string == "\u{65e5}\u{672c}\u{8a9e}" else { return -8 }
    guard field.accessibilityRole() == .textField, field.accessibilityIdentifier() == id else { return -9 }
    editor.insertTab(nil)
    guard let delegate = editor.delegate as? NSView, delegate !== field else { return -10 }
    editor.insertBacktab(nil)
    guard editor.delegate === field else { return -11 }
    return 1
}

@_cdecl("w3_post_key") @MainActor
public func postKey(_ pointer: UnsafeMutableRawPointer, _ idBytes: UnsafePointer<UInt8>?, _ idCount: Int) -> Int32 {
    let value = host(pointer)
    guard let field = value.nodes[string(idBytes, idCount)] as? NSTextField,
          value.window.makeFirstResponder(field),
          let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [],
            timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: value.window.windowNumber,
            context: nil, characters: "x", charactersIgnoringModifiers: "x", isARepeat: false, keyCode: 7) else { return 0 }
    NSApp.postEvent(event, atStart: false)
    return 1
}

@_cdecl("w3_shortcut") @MainActor
public func performShortcut(_ pointer: UnsafeMutableRawPointer, _ bytes: UnsafePointer<UInt8>?, _ count: Int) -> Int32 {
    let value = host(pointer)
    let (key, flags) = shortcut(string(bytes, count))
    guard let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: flags,
        timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: value.window.windowNumber,
        context: nil, characters: key, charactersIgnoringModifiers: key, isARepeat: false, keyCode: 0) else { return 0 }
    return NSApp.mainMenu?.performKeyEquivalent(with: event) == true ? 1 : 0
}

@_cdecl("w3_capture") @MainActor
public func capture(_ pointer: UnsafeMutableRawPointer, _ pathBytes: UnsafePointer<UInt8>?, _ pathCount: Int, _ width: Double) -> Int32 {
    let value = host(pointer)
    value.window.setContentSize(NSSize(width: width, height: 660))
    value.window.displayIfNeeded()
    guard let content = value.window.contentView,
          let bitmap = content.bitmapImageRepForCachingDisplay(in: content.bounds) else { return 0 }
    content.cacheDisplay(in: content.bounds, to: bitmap)
    guard let data = bitmap.representation(using: .png, properties: [:]) else { return 0 }
    do { try data.write(to: URL(fileURLWithPath: string(pathBytes, pathCount))); return 1 }
    catch { return 0 }
}