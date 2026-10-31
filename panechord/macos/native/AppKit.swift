import AppKit

public typealias Callback = @convention(c) (UnsafeMutableRawPointer?, Int32, UnsafePointer<UInt8>?, Int, UnsafePointer<UInt8>?, Int) -> Void

func string(_ bytes: UnsafePointer<UInt8>?, _ count: Int) -> String {
    guard let bytes = bytes else { return "" }
    return String(decoding: UnsafeBufferPointer(start: bytes, count: count), as: UTF8.self)
}

struct TableDefinition: Decodable {
    struct Column: Decodable {
        let title: String
        let width: Double?
        let role: Int
        let scale: Double
        let foreground: [Int]?
        let overflow: Int
        let stretch: Bool
        var font: NSFont {
            let size = NSFont.systemFontSize * scale
            return role == 2 ? NSFont.monospacedSystemFont(ofSize: size, weight: .regular)
                : role == 1 ? NSFont.boldSystemFont(ofSize: size * 1.2) : NSFont.systemFont(ofSize: size)
        }
    }
    struct Cell: Decodable { let text: String; let foreground: [Int]? }
    struct Row: Decodable { let key: String; let cells: [Cell] }
    let columns: [Column]
    let rows: [Row]
    let emptyText: String
    let columnGap: Double
    let rowGap: Double
}

@MainActor
final class SelectableTableCell: NSTableCellView {
    weak var owner: TableControl?
    var rowKey = ""
    var column = 0
    override func mouseDown(with event: NSEvent) {
        if owner?.selectionPresent == true { owner?.pick(self) }
        else { super.mouseDown(with: event) }
    }
}

struct TableCursor: Decodable { let row_key: String; let column: Int }

@MainActor
final class TableControl: NSScrollView, NSTableViewDataSource, NSTableViewDelegate {
    let table = NSTableView()
    let empty = NSTextField(labelWithString: "")
    var definition: TableDefinition?
    weak var owner: Host?
    var key = ""
    var cursor: TableCursor?
    var selectionPresent = false
    var selectionEnabled = false

    func setCursor(_ text: String) {
        guard let data = text.data(using: .utf8), let value = try? JSONDecoder().decode(TableCursor?.self, from: data) else {
            if text == "null" { cursor = nil; table.reloadData() }
            return
        }
        cursor = value
        table.reloadData()
    }

    func pick(_ cell: SelectableTableCell) {
        guard selectionEnabled, let owner, !owner.closed, owner.interactive(key), owner.nodes[key] === self,
              table.row(for: cell) >= 0, let definition,
              definition.columns.indices.contains(cell.column), definition.rows.contains(where: { $0.key == cell.rowKey }),
              let data = try? JSONSerialization.data(withJSONObject: [cell.rowKey, cell.column]),
              let value = String(data: data, encoding: .utf8) else { return }
        owner.emit(key, value)
    }

    init() {
        super.init(frame: NSRect(x: 0, y: 0, width: 480, height: 264))
        hasVerticalScroller = true
        autohidesScrollers = true
        drawsBackground = false
        documentView = table
        table.dataSource = self
        table.delegate = self
        table.allowsEmptySelection = true
        table.allowsColumnReordering = false
        table.columnAutoresizingStyle = .lastColumnOnlyAutoresizingStyle
        contentView.addSubview(empty)
        empty.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([empty.centerXAnchor.constraint(equalTo: contentView.centerXAnchor),
            empty.centerYAnchor.constraint(equalTo: contentView.centerYAnchor)])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }

    func update(_ text: String) {
        guard let bytes = text.data(using: .utf8), let next = try? JSONDecoder().decode(TableDefinition.self, from: bytes) else { return }
        let origin = contentView.bounds.origin
        let columnsChanged = definition?.columns.map { $0.title } != next.columns.map { $0.title }
            || definition?.columns.map { $0.width } != next.columns.map { $0.width }
        definition = next
        let headers = next.columns.contains { !$0.title.isEmpty }
        if headers != (table.headerView != nil) {
            table.headerView = headers ? NSTableHeaderView(frame: NSRect(x: 0, y: 0, width: 480, height: 24)) : nil
        }
        empty.stringValue = next.emptyText
        empty.isHidden = !next.rows.isEmpty
        if columnsChanged {
            for column in table.tableColumns { table.removeTableColumn(column) }
            for (index, value) in next.columns.enumerated() {
                let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier(String(index)))
                column.title = value.title
                column.headerCell.font = NSFont.boldSystemFont(ofSize: NSFont.systemFontSize)
                let natural = next.rows.reduce((value.title as NSString).size(withAttributes: [.font: column.headerCell.font!]).width) { width, row in
                    max(width, (row.cells[index].text as NSString).size(withAttributes: [.font: value.font]).width)
                }
                column.width = value.width ?? (value.stretch ? 120 : min(400, natural + 4))
                column.minWidth = value.width ?? 24
                column.resizingMask = value.stretch ? .autoresizingMask : []
                table.addTableColumn(column)
            }
        }
        table.intercellSpacing = NSSize(width: next.columnGap, height: next.rowGap)
        table.reloadData()
        contentView.scroll(to: origin)
        reflectScrolledClipView(contentView)
    }
    func numberOfRows(in tableView: NSTableView) -> Int { definition?.rows.count ?? 0 }
    func tableView(_ tableView: NSTableView, shouldSelectRow row: Int) -> Bool { false }
    func tableView(_ tableView: NSTableView, heightOfRow row: Int) -> CGFloat {
        guard let definition = definition else { return 16 }
        return definition.columns.enumerated().reduce(CGFloat(selectionPresent ? 34 : 16)) { height, entry in
            let (index, column) = entry
            guard column.overflow == 1 else { return max(height, column.font.ascender - column.font.descender) }
            let width = max(1, tableView.tableColumns[index].width - 4)
            let measured = (definition.rows[row].cells[index].text as NSString).boundingRect(
                with: NSSize(width: width, height: .greatestFiniteMagnitude), options: [.usesLineFragmentOrigin], attributes: [.font: column.font])
            return max(height, ceil(measured.height))
        }
    }
    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard let definition = definition, let tableColumn = tableColumn,
            let index = Int(tableColumn.identifier.rawValue) else { return nil }
        let column = definition.columns[index]
        let value = definition.rows[row].cells[index]
        let cell = (tableView.makeView(withIdentifier: tableColumn.identifier, owner: self) as? SelectableTableCell) ?? SelectableTableCell()
        cell.owner = self
        cell.rowKey = definition.rows[row].key
        cell.column = index
        cell.identifier = tableColumn.identifier
        cell.setAccessibilityIdentifier(definition.rows[row].key + "/" + String(index))
        let field = cell.textField ?? NSTextField(wrappingLabelWithString: "")
        if cell.textField == nil {
            cell.textField = field
            cell.addSubview(field)
            field.translatesAutoresizingMaskIntoConstraints = false
            NSLayoutConstraint.activate([field.leadingAnchor.constraint(equalTo: cell.leadingAnchor),
                field.trailingAnchor.constraint(equalTo: cell.trailingAnchor), field.topAnchor.constraint(equalTo: cell.topAnchor)])
        }
        field.stringValue = value.text
        field.isSelectable = !selectionPresent
        field.font = column.font
        field.maximumNumberOfLines = column.overflow == 1 ? 0 : 1
        field.lineBreakMode = column.overflow == 2 ? .byTruncatingMiddle : column.overflow == 1 ? .byWordWrapping : .byClipping
        let rgb = value.foreground ?? column.foreground
        let active = cursor?.row_key == cell.rowKey && cursor?.column == index
        cell.wantsLayer = true
        cell.layer?.backgroundColor = active ? NSColor.selectedContentBackgroundColor.cgColor : NSColor.clear.cgColor
        field.textColor = active ? .selectedTextColor : rgb.map { NSColor(srgbRed: Double($0[0]) / 255, green: Double($0[1]) / 255, blue: Double($0[2]) / 255, alpha: 1) } ?? .labelColor
        return cell
    }
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
final class Scrolling: NSScrollView {
    let body = DocumentStack()

    init(horizontal: Bool) {
        super.init(frame: .zero)
        hasHorizontalScroller = horizontal
        hasVerticalScroller = !horizontal
        autohidesScrollers = false
        drawsBackground = false
        body.orientation = .vertical
        body.alignment = .leading
        body.spacing = 0
        documentView = body
        body.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            body.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
            body.topAnchor.constraint(equalTo: contentView.topAnchor),
            horizontal
                ? body.heightAnchor.constraint(equalTo: contentView.heightAnchor)
                : body.widthAnchor.constraint(equalTo: contentView.widthAnchor)
        ])
    }

    required init?(coder: NSCoder) { fatalError("coder construction unsupported") }
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
final class TabButton: NSButton {
    weak var tabs: Tabs?
    var page = 0
    @objc func selectPage(_ sender: Any?) { tabs?.body.selectTabViewItem(at: page) }
}

@MainActor
final class Tabs: NSStackView {
    let header = NSSegmentedControl()
    let body = NSTabView()
    let verticalHeader = DocumentStack()
    let rail = NSScrollView()
    var enabled = true
    private lazy var bodyWidth = body.widthAnchor.constraint(equalTo: widthAnchor)
    private lazy var verticalBodyWidth = body.widthAnchor.constraint(equalTo: widthAnchor, constant: -128)
    private lazy var bodyHeight = body.heightAnchor.constraint(equalTo: heightAnchor)
    private lazy var railWidth = rail.widthAnchor.constraint(equalToConstant: 120)
    private lazy var railHeight = rail.heightAnchor.constraint(equalTo: heightAnchor)
    override init(frame: NSRect) {
        super.init(frame: frame)
        orientation = .vertical
        alignment = .leading
        spacing = 8
        header.trackingMode = .selectOne
        header.segmentStyle = .rounded
        body.tabViewType = .noTabsNoBorder
        distribution = .fill
        body.setContentHuggingPriority(.defaultLow, for: .vertical)
        header.setContentHuggingPriority(.defaultHigh, for: .vertical)
        addArrangedSubview(header)
        addArrangedSubview(body)
        body.translatesAutoresizingMaskIntoConstraints = false
        verticalHeader.orientation = .vertical
        verticalHeader.alignment = .leading
        verticalHeader.spacing = 8
        verticalHeader.translatesAutoresizingMaskIntoConstraints = false
        rail.drawsBackground = false
        rail.hasVerticalScroller = true
        rail.translatesAutoresizingMaskIntoConstraints = false
        rail.documentView = verticalHeader
        verticalHeader.widthAnchor.constraint(equalTo: rail.contentView.widthAnchor).isActive = true
        setPosition("Top")
    }
    func setPosition(_ position: String) {
        let vertical = position == "Left" || position == "Right"
        bodyWidth.isActive = false
        verticalBodyWidth.isActive = false
        bodyHeight.isActive = false
        railWidth.isActive = false
        railHeight.isActive = false
        for view in arrangedSubviews { removeArrangedSubview(view); view.removeFromSuperview() }
        orientation = vertical ? .horizontal : .vertical
        alignment = vertical ? .top : .leading
        let headings: NSView = vertical ? rail : header
        if position == "Bottom" || position == "Right" {
            addArrangedSubview(body); addArrangedSubview(headings)
        } else { addArrangedSubview(headings); addArrangedSubview(body) }
        bodyWidth.isActive = !vertical
        verticalBodyWidth.isActive = vertical
        bodyHeight.isActive = vertical
        railWidth.isActive = vertical
        railHeight.isActive = vertical
    }
    func rebuildHeader() {
        header.segmentCount = body.numberOfTabViewItems
        for view in verticalHeader.arrangedSubviews { verticalHeader.removeArrangedSubview(view); view.removeFromSuperview() }
        for (index, item) in body.tabViewItems.enumerated() {
            header.setLabel(item.label, forSegment: index)
            let button = TabButton(title: item.label, target: nil, action: nil)
            button.tabs = self
            button.page = index
            button.target = button
            button.action = #selector(TabButton.selectPage(_:))
            button.setButtonType(.pushOnPushOff)
            button.bezelStyle = .rounded
            button.cell?.alignment = .left
            button.cell?.wraps = true
            button.isEnabled = enabled
            verticalHeader.addArrangedSubview(button)
            button.translatesAutoresizingMaskIntoConstraints = false
            button.widthAnchor.constraint(equalTo: verticalHeader.widthAnchor).isActive = true
        }
        refreshSelection()
        verticalHeader.setFrameSize(NSSize(width: 120, height: verticalHeader.fittingSize.height))
    }
    func activatePage(_ index: Int) {
        if orientation == .horizontal, let button = verticalHeader.arrangedSubviews[index] as? NSButton {
            button.performClick(nil)
        } else { body.selectTabViewItem(at: index) }
        refreshSelection()
    }
    func refreshSelection() {
        let selected = body.selectedTabViewItem.map { body.indexOfTabViewItem($0) } ?? -1
        header.selectedSegment = selected
        for (index, view) in verticalHeader.arrangedSubviews.enumerated() {
            (view as? NSButton)?.state = index == selected ? .on : .off
        }
    }
    func setEnabled(_ enabled: Bool) {
        self.enabled = enabled
        header.isEnabled = enabled
        for view in verticalHeader.arrangedSubviews { (view as? NSButton)?.isEnabled = enabled }
    }
    required init?(coder: NSCoder) { fatalError("coder construction unsupported") }
}

@MainActor
final class PointerImage: NSImageView {
    weak var owner: Host?
    var key = ""
    var pointerEnabled = false
    private var pointerArea: NSTrackingArea?
    override var isFlipped: Bool { true }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let area = pointerArea { removeTrackingArea(area) }
        let area = NSTrackingArea(rect: bounds, options: [.mouseMoved, .mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect], owner: self)
        addTrackingArea(area)
        pointerArea = area
    }

    override func mouseMoved(with event: NSEvent) {
        guard pointerEnabled, let owner, !owner.closed, owner.interactive(key) else { return }
        let point = convert(event.locationInWindow, from: nil)
        guard let data = try? JSONSerialization.data(withJSONObject: [point.x, point.y]),
              let value = String(data: data, encoding: .utf8) else { return }
        owner.emit(key, value)
    }

    override func mouseExited(with event: NSEvent) {
        guard pointerEnabled, let owner, !owner.closed, owner.interactive(key) else { return }
        owner.emit(key, "null")
    }
}

@MainActor
final class Host: NSObject, NSWindowDelegate, NSTextFieldDelegate, NSTabViewDelegate {
    let window: NSWindow
    let root = DocumentStack()
    var nodes: [String: NSView] = [:]
    var parents: [String: String] = [:]
    var visibility: [String: Bool] = [:]
    var enablement: [String: Bool] = [:]
    var capabilities: [String: Bool] = [:]
    var frames: [String: NSStackView] = [:]
    var dimensions: [String: [NSLayoutConstraint]] = [:]
    var contentWidths: [String: NSLayoutConstraint] = [:]
    var contentHeights: [String: NSLayoutConstraint] = [:]
    var menus: [String: NSMenu] = [:]
    var menuItems: [String: NSMenuItem] = [:]
    var tabItems: [String: NSTabViewItem] = [:]
    var dialogs: [String: NSPanel] = [:]
    var readonlyTabs: Set<String> = []
    var readonlyDialogs: Set<String> = []
    var booleanButtons: Set<String> = []
    var applying = false
    var focusable: [NSView] = []
    var callback: Callback?
    var context: UnsafeMutableRawPointer?
    var closed = false
    weak var editingProbe: NSTextView?
    var editingProbeText = ""
    var inputMonitor: Any?
    var pressedCodes = Set<UInt16>()
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
        let appMenu = NSMenu(title: "Panechord")
        app.submenu = appMenu
        bar.addItem(app)
        let about = NSMenuItem(title: "About Panechord", action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)), keyEquivalent: "")
        appMenu.addItem(about)
        appMenu.addItem(.separator())
        let quit = NSMenuItem(title: "Quit Panechord", action: #selector(ApplicationDelegate.quitRequested), keyEquivalent: "q")
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
        inputMonitor = NSEvent.addLocalMonitorForEvents(matching: [.keyDown, .keyUp, .flagsChanged]) { [weak self] event in
            guard let self, !self.closed, event.window === self.window else { return event }
            let pressed: Bool
            if event.type == .flagsChanged {
                pressed = !self.pressedCodes.contains(event.keyCode)
            } else {
                pressed = event.type == .keyDown
            }
            if pressed { self.pressedCodes.insert(event.keyCode) }
            else { self.pressedCodes.remove(event.keyCode) }
            let repeated = event.type == .keyDown && event.isARepeat
            self.emit("", String(event.keyCode), kind: pressed ? (repeated ? 7 : 5) : 6)
            return event
        }
    }

    func stopInput() {
        if let inputMonitor { NSEvent.removeMonitor(inputMonitor) }
        inputMonitor = nil
        pressedCodes.removeAll()
    }

    func windowDidBecomeKey(_ notification: Notification) {
        guard !closed else { return }
        ApplicationDelegate.shared.active = self
        NSApp.mainMenu = bar
        emit("", "1", kind: 4)
    }

    func windowDidResignKey(_ notification: Notification) {
        pressedCodes.removeAll()
        emit("", "0", kind: 4)
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
        if let dialog = dialogs.first(where: { $0.value === sender }) {
            if readonlyDialogs.contains(dialog.key) { return false }
            emit(dialog.key, "0")
            return false
        }
        emit("", kind: 1)
        return false
    }
    @objc func activated(_ sender: NSControl) {
        guard !applying, let id = sender.identifier?.rawValue else { return }
        if let slider = sender as? NSSlider { emit(id, String(slider.doubleValue)) }
        else if let header = sender as? NSSegmentedControl, let tabs = nodes[id] as? Tabs,
            header.selectedSegment >= 0, header.selectedSegment < tabs.body.numberOfTabViewItems {
            tabs.body.selectTabViewItem(at: header.selectedSegment)
        }
        else if let combo = sender as? NSPopUpButton { emit(id, combo.selectedItem?.representedObject as? String ?? "") }
        else if let button = sender as? NSButton, booleanButtons.contains(id) {
            emit(id, button.state == .on ? "1" : "0")
        } else { emit(id) }
    }
    func tabView(_ tabView: NSTabView, shouldSelect tabViewItem: NSTabViewItem?) -> Bool {
        applying || !readonlyTabs.contains(tabView.identifier?.rawValue ?? "")
    }
    func tabView(_ tabView: NSTabView, didSelect tabViewItem: NSTabViewItem?) {
        if let id = tabView.identifier?.rawValue, let tabs = nodes[id] as? Tabs, tabViewItem != nil {
            tabs.refreshSelection()
        }
        if !applying, let id = tabView.identifier?.rawValue, let key = tabViewItem?.identifier as? String { emit(id, key) }
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

    func attach(_ view: NSView, parent: String, id: String, floating: Bool = false) {
        let container = (nodes[parent] as? Rows)?.body ?? (nodes[parent] as? Scrolling)?.body ?? nodes[parent] as? NSStackView
        let tab = (nodes[parent] as? Tabs)?.body
        guard floating || container != nil || tab != nil else { return }
        let frame = NSStackView()
        frame.orientation = .vertical
        frame.alignment = .leading
        frame.spacing = 0
        frame.identifier = NSUserInterfaceItemIdentifier("frame/" + id)
        frame.addArrangedSubview(view)
        if floating {
            let panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 480, height: 320),
                styleMask: [.titled, .closable], backing: .buffered, defer: false)
            panel.isReleasedWhenClosed = false
            panel.delegate = self
            panel.contentView = frame
            dialogs[id] = panel
        } else if let tab = tab {
            let item = NSTabViewItem(identifier: id)
            item.view = frame
            item.label = id
            applying = true
            tab.addTabViewItem(item)
            applying = false
            tabItems[id] = item
            if let tabs = nodes[parent] as? Tabs {
                tabs.rebuildHeader()
            }
        } else { container?.addArrangedSubview(frame) }
        frame.translatesAutoresizingMaskIntoConstraints = false
        if floating || tab != nil {
            frame.translatesAutoresizingMaskIntoConstraints = true
            frame.autoresizingMask = [.width, .height]
        }
        view.translatesAutoresizingMaskIntoConstraints = false
        let contentWidth = view.widthAnchor.constraint(equalTo: frame.widthAnchor, constant: 0)
        contentWidth.isActive = true
        let contentHeight = view.heightAnchor.constraint(equalTo: frame.heightAnchor)
        contentHeight.isActive = view is Tabs || view is Scrolling || view is TableControl
        contentWidths[id] = contentWidth
        contentHeights[id] = contentHeight
        if let container = container, container.orientation == .vertical, !floating, tab == nil {
            frame.widthAnchor.constraint(equalTo: container.widthAnchor,
                constant: -(container.edgeInsets.left + container.edgeInsets.right)).isActive = true
        }
        nodes[id] = view
        parents[id] = parent
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
    func interactive(_ id: String) -> Bool {
        var current: String? = id
        while let key = current {
            if visibility[key] == false || enablement[key] == false { return false }
            current = parents[key]
        }
        return true
    }
    func refreshEnablement() {
        for (id, view) in nodes {
            let enabled = interactive(id) && capabilities[id] != false
            (view as? NSControl)?.isEnabled = enabled
            (view as? Tabs)?.setEnabled(enabled)
        }
    }

    func remove(_ prefix: String) {
        let previousApplying = applying
        applying = true
        defer { applying = previousApplying }
        let ids = nodes.keys.filter { $0 == prefix || $0.hasPrefix(prefix + "/") }
        for id in ids.sorted(by: { $0.count > $1.count }) {
            if let panel = dialogs.removeValue(forKey: id) {
                if panel.sheetParent != nil { window.endSheet(panel) }
                panel.orderOut(nil)
            }
            if let item = tabItems.removeValue(forKey: id) { item.tabView?.removeTabViewItem(item) }
            readonlyTabs.remove(id)
            readonlyDialogs.remove(id)
            booleanButtons.remove(id)
            if let frame = frames.removeValue(forKey: id) {
                if let parent = frame.superview as? NSStackView { parent.removeArrangedSubview(frame) }
                frame.removeFromSuperview()
            }
            dimensions.removeValue(forKey: id)
            contentWidths.removeValue(forKey: id)
            contentHeights.removeValue(forKey: id)
            parents.removeValue(forKey: id)
            visibility.removeValue(forKey: id)
            enablement.removeValue(forKey: id)
            capabilities.removeValue(forKey: id)
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
    value.stopInput()
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
    value.stopInput()
    for panel in value.dialogs.values {
        if panel.sheetParent != nil { value.window.endSheet(panel) }
        panel.orderOut(nil)
    }
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
    case 6, 7:
        value.booleanButtons.insert(id)
        let button = NSButton(title: "", target: value, action: #selector(Host.activated(_:)))
        button.setButtonType(kind == 6 ? .switch : .radio)
        view = button
    case 8:
        let slider = NSSlider(value: 0, minValue: 0, maxValue: 100, target: value, action: #selector(Host.activated(_:)))
        slider.isContinuous = true
        view = slider
    case 9:
        let combo = NSPopUpButton(frame: .zero, pullsDown: false)
        combo.target = value
        combo.action = #selector(Host.activated(_:))
        combo.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        view = combo
    case 10:
        let tabs = Tabs(frame: NSRect(x: 0, y: 0, width: 420, height: 180))
        tabs.body.delegate = value
        tabs.body.identifier = NSUserInterfaceItemIdentifier(id)
        tabs.header.identifier = NSUserInterfaceItemIdentifier(id)
        tabs.header.target = value
        tabs.header.action = #selector(Host.activated(_:))
        view = tabs
    case 11, 12:
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 8
        view = stack
    case 13:
        let image = PointerImage()
        image.owner = value
        image.key = id
        image.imageScaling = .scaleProportionallyUpOrDown
        image.imageAlignment = .alignCenter
        view = image
    case 15:
        view = Scrolling(horizontal: number != 0)
    case 16:
        let separator = NSBox()
        separator.boxType = .separator
        separator.setAccessibilityRole(.splitter)
        if number != 0 {
            separator.widthAnchor.constraint(equalToConstant: 1).isActive = true
        } else {
            separator.heightAnchor.constraint(equalToConstant: 1).isActive = true
        }
        view = separator
    case 17:
        let table = TableControl()
        table.owner = value
        table.key = id
        view = table
    default:
        let rows = Rows()
        rows.changed = { [weak value] start in value?.emit(id, String(start), kind: 2) }
        view = rows
    }
    value.attach(view, parent: string(parentBytes, parentCount), id: id, floating: kind == 12)
}

@_cdecl("w3_set") @MainActor
public func set(_ pointer: UnsafeMutableRawPointer, _ idBytes: UnsafePointer<UInt8>?, _ idCount: Int,
         _ textBytes: UnsafePointer<UInt8>?, _ textCount: Int, _ property: Int32) {
    let value = host(pointer)
    let id = string(idBytes, idCount)
    let text = string(textBytes, textCount)
    if property == 18, let table = value.nodes[id] as? TableControl {
        table.selectionPresent = true
        table.selectionEnabled = text == "1"
        table.table.reloadData()
    } else if property == 17, let table = value.nodes[id] as? TableControl {
        table.setCursor(text)
    } else if property == 16, let stack = value.nodes[id] as? NSStackView,
       let data = text.data(using: .utf8), let keys = try? JSONDecoder().decode([String].self, from: data) {
        for key in keys {
            guard let frame = value.frames[key], stack.arrangedSubviews.contains(frame) else { continue }
            stack.removeArrangedSubview(frame)
            stack.addArrangedSubview(frame)
        }
    } else if property == 15, let image = value.nodes[id] as? PointerImage {
        image.pointerEnabled = text == "1"
    } else if property == 14, let field = value.nodes[id] as? NSTextField {
        field.placeholderString = text
    } else if property == 13, let table = value.nodes[id] as? TableControl {
        table.update(text)
    } else if property == 1 {
        value.capabilities[id] = text == "1"
        (value.nodes[id] as? NSControl)?.isEnabled = text == "1"
        if value.dialogs[id] != nil {
            if text == "1" { value.readonlyDialogs.remove(id) } else { value.readonlyDialogs.insert(id) }
        }
        if let tabs = value.nodes[id] as? Tabs {
            tabs.setEnabled(text == "1")
            if text == "1" { value.readonlyTabs.remove(id) } else { value.readonlyTabs.insert(id) }
        }
        value.refreshEnablement()
    } else if property == 11 {
        value.visibility[id] = text == "1"
        value.frames[id]?.isHidden = text != "1"
        value.refreshEnablement()
    } else if property == 12 {
        value.enablement[id] = text == "1"
        value.refreshEnablement()
    } else if property == 2 {
        value.window.title = text
    } else if property == 3 {
        (value.nodes[id] as? NSButton)?.state = text == "1" ? .on : .off
    } else if property == 4, let slider = value.nodes[id] as? NSSlider,
        let data = text.data(using: .utf8), let config = try? JSONSerialization.jsonObject(with: data) as? [String: Double],
        let min = config["min"], let max = config["max"], min.isFinite, max.isFinite, min < max {
        slider.minValue = min
        slider.maxValue = max
    } else if property == 5, let combo = value.nodes[id] as? NSPopUpButton,
        let data = text.data(using: .utf8), let choices = try? JSONSerialization.jsonObject(with: data) as? [[String: String]] {
        combo.removeAllItems()
        for choice in choices {
            combo.addItem(withTitle: choice["label"] ?? "")
            combo.lastItem?.representedObject = choice["id"] ?? ""
        }
    } else if property == 6 {
        value.applying = true
        defer { value.applying = false }
        if let slider = value.nodes[id] as? NSSlider, let number = Double(text) { slider.doubleValue = number }
        if let combo = value.nodes[id] as? NSPopUpButton {
            combo.select(combo.itemArray.first { ($0.representedObject as? String) == text })
        }
        if let tab = (value.nodes[id] as? Tabs)?.body {
            if let item = tab.tabViewItems.first(where: { ($0.identifier as? String) == text }) { tab.selectTabViewItem(item) }
            else if text.isEmpty, tab.numberOfTabViewItems > 0 { tab.selectTabViewItem(at: 0) }
        }
    } else if property == 7, let panel = value.dialogs[id] {
        if text == "1", !panel.isVisible {
            if value.window.isVisible { value.window.beginSheet(panel) } else { panel.makeKeyAndOrderFront(nil) }
        } else if text != "1" {
            if panel.sheetParent != nil { value.window.endSheet(panel) }
            panel.orderOut(nil)
        }
    } else if property == 8 { value.tabItems[id]?.identifier = text
    } else if property == 9 { value.dialogs[id]?.title = text
    } else if property == 10, let tabs = value.nodes[id] as? Tabs { tabs.setPosition(text)
    } else if let item = value.tabItems[id] {
        item.label = text
        if let tab = item.tabView, let id = tab.identifier?.rawValue, let tabs = value.nodes[id] as? Tabs {
            tabs.rebuildHeader()
        }
    } else if let field = value.nodes[id] as? NSTextField {
        if let editor = field.currentEditor() as? NSTextView, editor.hasMarkedText() { return }
        if field.stringValue != text { field.stringValue = text }
    } else if let button = value.nodes[id] as? NSButton { button.title = text }
}

@_cdecl("w3_slider_value") @MainActor
public func sliderValue(_ pointer: UnsafeMutableRawPointer, _ idBytes: UnsafePointer<UInt8>?, _ idCount: Int,
                        _ output: UnsafeMutablePointer<Double>) -> Bool {
    guard let slider = host(pointer).nodes[string(idBytes, idCount)] as? NSSlider else { return false }
    output.pointee = slider.doubleValue
    return true
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
    if !value.interactive(id) { return 0 }
    if kind == 7, let slider = value.nodes[id] as? NSSlider, slider.isEnabled { return 1 }
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
    let text = string(textBytes, textCount)
    if kind == 3, let button = value.nodes[id] as? NSButton, button.isEnabled, text == "0" || text == "1" {
        button.state = text == "1" ? .on : .off
        value.activated(button)
        return 1
    }
    if kind == 4, let slider = value.nodes[id] as? NSSlider, slider.isEnabled,
       let number = Double(text), number.isFinite, (slider.minValue...slider.maxValue).contains(number) {
        slider.doubleValue = number
        value.activated(slider)
        return 1
    }
    if kind == 5, let combo = value.nodes[id] as? NSPopUpButton, combo.isEnabled,
       let item = combo.itemArray.first(where: { ($0.representedObject as? String) == text }) {
        combo.select(item)
        value.activated(combo)
        return 1
    }
    if kind == 5, let tabs = value.nodes[id] as? Tabs, !value.readonlyTabs.contains(id),
       let item = tabs.body.tabViewItems.first(where: { ($0.identifier as? String) == text }) {
        tabs.activatePage(tabs.body.indexOfTabViewItem(item))
        return 1
    }
    if kind == 6, value.dialogs[id] != nil, !value.readonlyDialogs.contains(id), text == "0" || text == "1" {
        value.emit(id, text)
        return 1
    }
    return 0
}

@_cdecl("w3_image") @MainActor
public func image(_ pointer: UnsafeMutableRawPointer, _ idBytes: UnsafePointer<UInt8>?, _ idCount: Int,
                  _ pixels: UnsafePointer<UInt8>?, _ length: Int, _ width: UInt32, _ height: UInt32) -> Int32 {
    guard let imageView = host(pointer).nodes[string(idBytes, idCount)] as? NSImageView else { return 0 }
    if width == 0, height == 0, length == 0 { imageView.image = nil; return 1 }
    guard width > 0, height > 0, width <= 8192, height <= 8192,
          length >= 0, UInt64(width) * UInt64(height) * 4 == UInt64(length), length <= 64 * 1024 * 1024,
          let pixels = pixels,
          let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(width), pixelsHigh: Int(height),
              bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
              bitmapFormat: .alphaNonpremultiplied, bytesPerRow: Int(width) * 4, bitsPerPixel: 32),
          let destination = bitmap.bitmapData else { return 0 }
    destination.update(from: pixels, count: length)
    let image = NSImage(size: NSSize(width: Int(width), height: Int(height)))
    image.addRepresentation(bitmap)
    imageView.image = image
    return 1
}

@_cdecl("w3_style") @MainActor
public func style(_ pointer: UnsafeMutableRawPointer, _ idBytes: UnsafePointer<UInt8>?, _ idCount: Int,
           _ top: Double, _ right: Double, _ bottom: Double, _ left: Double,
           _ width: Double, _ height: Double, _ horizontal: Int32, _ vertical: Int32,
           _ textRole: Int32, _ textScale: Double, _ red: Int32, _ green: Int32, _ blue: Int32) {
    let value = host(pointer)
    let id = string(idBytes, idCount)
    guard let frame = value.frames[id] else { return }
    if let label = value.nodes[id] as? NSTextField, !label.isEditable {
        let size = NSFont.systemFontSize * textScale
        label.font = textRole == 3 ? NSFont.monospacedSystemFont(ofSize: size, weight: .regular)
            : textRole == 2 ? NSFont.boldSystemFont(ofSize: size * 1.2) : NSFont.systemFont(ofSize: size)
        label.textColor = red >= 0 && green >= 0 && blue >= 0
            ? NSColor(srgbRed: Double(red) / 255, green: Double(green) / 255, blue: Double(blue) / 255, alpha: 1)
            : NSColor.labelColor
    }
    if let constraints = value.dimensions[id] { NSLayoutConstraint.deactivate(constraints) }
    value.contentWidths[id]?.isActive = false
    value.contentHeights[id]?.isActive = false
    frame.edgeInsets = NSEdgeInsets(top: top, left: left, bottom: bottom, right: right)
    value.contentWidths[id]?.constant = -(left + right)
    value.contentHeights[id]?.constant = -(top + bottom)
    frame.alignment = horizontal == 2 ? .centerX : horizontal == 3 ? .trailing : .leading
    let stretchWidth = horizontal == 0 || horizontal == 4
    if let node = value.nodes[id], node !== frame {
        frame.removeView(node)
        frame.addView(node, in: vertical == 2 ? .center : vertical == 3 ? .bottom : .top)
        value.contentHeights[id]?.isActive = vertical == 4 || (vertical == 0 && (node is Tabs || node is Scrolling))
    }
    value.contentWidths[id]?.isActive = stretchWidth
    frame.setHuggingPriority(vertical == 4 ? NSLayoutConstraint.Priority(rawValue: 1) : .defaultLow, for: .vertical)
    var constraints: [NSLayoutConstraint] = []
    if width >= 0 {
        if stretchWidth { constraints.append(frame.widthAnchor.constraint(equalToConstant: width)) }
        else if let node = value.nodes[id] { constraints.append(node.widthAnchor.constraint(equalToConstant: max(0, width - left - right))) }
    }
    if height >= 0 { constraints.append(frame.heightAnchor.constraint(equalToConstant: height)) }
    else if vertical > 0, vertical < 4, let container = frame.superview as? NSStackView, container.orientation == .horizontal {
        constraints.append(frame.heightAnchor.constraint(equalTo: container.heightAnchor,
            constant: -(container.edgeInsets.top + container.edgeInsets.bottom)))
    }
    NSLayoutConstraint.activate(constraints)
    value.dimensions[id] = constraints
}

@_cdecl("w3_remove") @MainActor
public func remove(_ pointer: UnsafeMutableRawPointer, _ idBytes: UnsafePointer<UInt8>?, _ idCount: Int) {
    host(pointer).remove(string(idBytes, idCount))
}

@_cdecl("w3_reorder") @MainActor
public func reorder(_ pointer: UnsafeMutableRawPointer, _ idBytes: UnsafePointer<UInt8>?, _ idCount: Int,
                    _ orderBytes: UnsafePointer<UInt8>?, _ orderCount: Int) -> Int32 {
    let value = host(pointer)
    let id = string(idBytes, idCount)
    guard let data = string(orderBytes, orderCount).data(using: .utf8),
          let ids = try? JSONSerialization.jsonObject(with: data) as? [String],
          Set(ids).count == ids.count else { return 0 }
    if let tabs = value.nodes[id] as? Tabs {
        let items = ids.compactMap { value.tabItems[$0] }
        guard items.count == ids.count, items.count == tabs.body.numberOfTabViewItems,
              items.allSatisfy({ $0.tabView === tabs.body }) else { return 0 }
        let selected = tabs.body.selectedTabViewItem
        let previousApplying = value.applying
        value.applying = true
        defer { value.applying = previousApplying }
        for (index, item) in items.enumerated() {
            if tabs.body.tabViewItem(at: index) !== item {
                tabs.body.removeTabViewItem(item)
                tabs.body.insertTabViewItem(item, at: index)
            }
        }
        if let selected = selected { tabs.body.selectTabViewItem(selected) }
        tabs.rebuildHeader()
        return 1
    }
    guard let rows = value.nodes[id] as? Rows,
          ids.count == rows.body.arrangedSubviews.count else { return 0 }
    let frames = ids.compactMap { value.frames[$0] }
    guard frames.count == ids.count, Set(ids).count == ids.count,
          frames.allSatisfy({ $0.superview === rows.body }) else { return 0 }
    for (index, frame) in frames.enumerated() {
        if rows.body.arrangedSubviews[index] !== frame {
            rows.body.insertArrangedSubview(frame, at: index)
        }
    }
    return 1
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

@_cdecl("w3_menu_tree") @MainActor
public func menuTree(_ pointer: UnsafeMutableRawPointer, _ idBytes: UnsafePointer<UInt8>?, _ idCount: Int,
                     _ jsonBytes: UnsafePointer<UInt8>?, _ jsonCount: Int) {
    let value = host(pointer)
    let id = string(idBytes, idCount)
    guard let root = value.menus[id], let data = string(jsonBytes, jsonCount).data(using: .utf8),
          let tree = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]] else { return }
    func build(_ menu: NSMenu, _ nodes: [[String: Any]]) {
        for node in nodes {
            if let children = node["children"] as? [[String: Any]] {
                let title = node["label"] as? String ?? ""
                let child = NSMenu(title: title)
                child.autoenablesItems = false
                let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
                item.isEnabled = node["enabled"] as? Bool ?? true
                item.submenu = child
                build(child, children)
                menu.addItem(item)
            } else if node["separator"] as? Bool == true { menu.addItem(.separator()) }
            else if let key = node["id"] as? String, let item = value.menuItems[id + "/" + key] {
                item.menu?.removeItem(item)
                menu.addItem(item)
            }
        }
    }
    root.removeAllItems()
    build(root, tree)
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

@_cdecl("w3_editing_probe") @MainActor
public func editingProbe(_ pointer: UnsafeMutableRawPointer, _ idBytes: UnsafePointer<UInt8>?, _ idCount: Int, _ begin: Int32) -> Int32 {
    let value = host(pointer)
    guard let field = value.nodes[string(idBytes, idCount)] as? NSTextField else { return 0 }
    if begin != 0 {
        value.window.makeFirstResponder(field)
        guard let editor = field.currentEditor() as? NSTextView else { return 0 }
        value.editingProbe = editor
        value.editingProbeText = field.stringValue
        editor.selectAll(nil)
        editor.setMarkedText("kana", selectedRange: NSRange(location: 2, length: 1), replacementRange: NSRange(location: NSNotFound, length: 0))
        return editor.hasMarkedText() ? 1 : 0
    }
    guard let editor = field.currentEditor() as? NSTextView, editor === value.editingProbe,
          editor.hasMarkedText(), editor.string == "kana", editor.selectedRange() == NSRange(location: 2, length: 1) else { return 0 }
    editor.setMarkedText(value.editingProbeText, selectedRange: NSRange(location: 0, length: 0), replacementRange: editor.markedRange())
    editor.unmarkText()
    value.editingProbe = nil
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