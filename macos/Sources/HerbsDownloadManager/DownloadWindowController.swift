import AppKit
import HDCore

/// The Download window (SPECIFICATION.md §6): the segments bar and numbers, Resume/Pause and Close, and the Info,
/// Connections, Mirrors and Options tabs. Closing it never stops the download.
@MainActor
final class DownloadWindowController: NSWindowController, NSWindowDelegate, NSTableViewDataSource,
    NSTableViewDelegate {
    let id: UInt64
    private var detail: DownloadDetail?
    private let bar = SegmentsBar()
    private let percent = NSTextField(labelWithString: "")
    private let numbers = NSTextField(labelWithString: "")
    private let toggle = NSButton(title: "Pause", target: nil, action: nil)
    private let tabs = NSTabView()
    private var info: [String: NSTextField] = [:]
    private let connectionsTable = NSTableView()
    private let sourcesTable = NSTableView()
    private let addConnection = NSButton(title: "+", target: nil, action: nil)
    private let removeConnection = NSButton(title: "−", target: nil, action: nil)
    private let removeMirror = NSButton(title: "Remove", target: nil, action: nil)
    private let optionConnections = NSPopUpButton()
    private var onClose: (() -> Void)?

    init(id: UInt64, onClose: @escaping () -> Void) {
        self.id = id
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 680, height: 470),
                              styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered,
                              defer: false)
        window.minSize = NSSize(width: 560, height: 380)
        window.setAccessibilityIdentifier("download-window")
        super.init(window: window)
        self.onClose = onClose
        window.delegate = self
        build()
        refresh()
        AppModel.shared.observe { [weak self] in self?.refresh() }
        window.center()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    func showTab(_ label: String) {
        if let item = tabs.tabViewItems.first(where: { $0.label == label }) { tabs.selectTabViewItem(item) }
    }

    // MARK: layout

    private func build() {
        guard let window else { return }
        bar.setAccessibilityLabel("Segments")
        bar.toolTip = "The file's ranges: filled parts are downloaded, gaps are still to fetch"
        percent.font = .monospacedDigitSystemFont(ofSize: 22, weight: .semibold)
        numbers.font = .monospacedDigitSystemFont(ofSize: NSFont.systemFontSize, weight: .regular)
        numbers.textColor = .secondaryLabelColor
        numbers.setAccessibilityIdentifier("numbers")

        toggle.target = self
        toggle.action = #selector(toggleRunning(_:))
        toggle.setAccessibilityIdentifier("toggle")
        let close = NSButton(title: "Close", target: self, action: #selector(closeWindow(_:)))
        close.toolTip = "Close this window; the download keeps going"
        let header = NSStackView(views: [percent, numbers, NSView(), toggle, close])
        header.spacing = 10

        tabs.addTabViewItem(tab("Info", infoView()))
        tabs.addTabViewItem(tab("Connections", connectionsView()))
        tabs.addTabViewItem(tab("Mirrors", mirrorsView()))
        tabs.addTabViewItem(tab("Options", optionsView()))

        let stack = NSStackView(views: [bar, header, tabs])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 10
        stack.edgeInsets = NSEdgeInsets(top: 16, left: 16, bottom: 16, right: 16)
        stack.translatesAutoresizingMaskIntoConstraints = false
        let content = NSView()
        content.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            stack.topAnchor.constraint(equalTo: content.topAnchor),
            stack.bottomAnchor.constraint(equalTo: content.bottomAnchor),
            bar.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -32),
            bar.heightAnchor.constraint(equalToConstant: 18),
            header.widthAnchor.constraint(equalTo: bar.widthAnchor),
            tabs.widthAnchor.constraint(equalTo: bar.widthAnchor),
        ])
        window.contentView = content
    }

    private func tab(_ label: String, _ view: NSView) -> NSTabViewItem {
        let item = NSTabViewItem(identifier: label)
        item.label = label
        item.view = view
        return item
    }

    private func padded(_ inner: NSView) -> NSView {
        let view = NSView()
        inner.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(inner)
        NSLayoutConstraint.activate([
            inner.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 12),
            inner.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -12),
            inner.topAnchor.constraint(equalTo: view.topAnchor, constant: 12),
            inner.bottomAnchor.constraint(lessThanOrEqualTo: view.bottomAnchor, constant: -12),
        ])
        return view
    }

    private func infoView() -> NSView {
        var rows: [[NSView]] = []
        for key in ["Link", "Folder", "Size", "Resumable", "Date Added", "Date Finished"] {
            let value = NSTextField(labelWithString: "")
            value.isSelectable = true
            value.lineBreakMode = .byTruncatingMiddle
            value.setAccessibilityIdentifier("info-\(key)")
            // Long links and folders are cut in the middle instead of widening the window.
            value.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
            value.widthAnchor.constraint(lessThanOrEqualToConstant: 520).isActive = true
            info[key] = value
            rows.append([NSTextField(labelWithString: "\(key):"), value])
        }
        let grid = NSGridView(views: rows)
        grid.column(at: 0).xPlacement = .trailing
        grid.rowSpacing = 8
        return padded(grid)
    }

    private func table(_ t: NSTableView, _ columns: [(String, String, CGFloat)]) -> NSScrollView {
        for (id, title, width) in columns {
            let c = NSTableColumn(identifier: .init(id))
            c.title = title
            c.width = width
            t.addTableColumn(c)
        }
        t.usesAlternatingRowBackgroundColors = true
        t.dataSource = self
        t.delegate = self
        let scroll = NSScrollView()
        scroll.documentView = t
        scroll.hasVerticalScroller = true
        scroll.borderType = .bezelBorder
        return scroll
    }

    private func connectionsView() -> NSView {
        let scroll = table(connectionsTable, [("host", "Source", 140), ("range", "Range", 150),
                                              ("downloaded", "Downloaded", 90), ("speed", "Speed", 80),
                                              ("state", "State", 120)])
        connectionsTable.setAccessibilityIdentifier("connections-table")
        addConnection.target = self
        addConnection.action = #selector(addOne(_:))
        addConnection.toolTip = "Add a connection now"
        addConnection.setAccessibilityLabel("Add Connection")
        removeConnection.target = self
        removeConnection.action = #selector(removeOne(_:))
        removeConnection.toolTip = "Remove a connection now"
        removeConnection.setAccessibilityLabel("Remove Connection")
        let buttons = NSStackView(views: [addConnection, removeConnection])
        return stacked(scroll, buttons)
    }

    private func mirrorsView() -> NSView {
        let scroll = table(sourcesTable, [("on", "", 24), ("url", "Address", 230), ("state", "State", 120),
                                          ("active", "Connections", 80), ("speed", "Speed", 80),
                                          ("bytes", "Served", 80)])
        sourcesTable.setAccessibilityIdentifier("sources-table")
        let add = NSButton(title: "Add Mirror…", target: self, action: #selector(addMirror(_:)))
        add.toolTip = "Add Mirror: another server with the same file (⌥⌘M)"
        removeMirror.target = self
        removeMirror.action = #selector(removeSelectedMirror(_:))
        removeMirror.toolTip = "Remove the selected mirror; the original link can only be turned off"
        let buttons = NSStackView(views: [add, removeMirror])
        return stacked(scroll, buttons)
    }

    private func stacked(_ top: NSView, _ bottom: NSView) -> NSView {
        let stack = NSStackView(views: [top, bottom])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 8
        let view = NSView()
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 10),
            stack.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -10),
            stack.topAnchor.constraint(equalTo: view.topAnchor, constant: 10),
            stack.bottomAnchor.constraint(equalTo: view.bottomAnchor, constant: -10),
            top.widthAnchor.constraint(equalTo: stack.widthAnchor),
        ])
        return view
    }

    private func optionsView() -> NSView {
        for n in connectionChoices() { optionConnections.addItem(withTitle: "\(n)") }
        optionConnections.target = self
        optionConnections.action = #selector(connectionsChosen(_:))
        optionConnections.toolTip = "Connections for this download (1–32), applied at once"
        optionConnections.setAccessibilityIdentifier("option-connections")
        let grid = NSGridView(views: [[NSTextField(labelWithString: "Connections:"), optionConnections]])
        grid.column(at: 0).xPlacement = .trailing
        return padded(grid)
    }

    // MARK: refresh

    private static let dates: DateFormatter = {
        let f = DateFormatter()
        f.dateStyle = .medium
        f.timeStyle = .short
        return f
    }()

    private func refresh() {
        guard let d = AppModel.shared.manager.detail(id: id) else { close(); return }
        detail = d
        let r = d.row
        window?.title = r.name
        bar.update(segments: d.segments, size: r.size ?? 0, finished: r.status.isFinished)
        percent.stringValue = r.fraction.map { "\(Int(($0 * 100).rounded(.down)))%" } ?? (r.status.isFinished ? "100%" : "")
        numbers.stringValue = [r.sizeText, r.speedText, r.timeLeftText.isEmpty ? "" : "\(r.timeLeftText) left", r.statusText]
            .filter { !$0.isEmpty }.joined(separator: "  ·  ")
        toggle.title = r.status.isRunning ? "Pause" : "Resume"
        toggle.isEnabled = r.status.isRunning || r.status.canResume
        toggle.toolTip = r.status.isRunning ? "Pause: stop, keeping what's downloaded (⌘.)" : "Resume: continue (⌘R)"

        info["Link"]?.stringValue = r.url
        info["Folder"]?.stringValue = (r.folder as NSString).abbreviatingWithTildeInPath
        info["Size"]?.stringValue = r.size.map { bytesText(bytes: $0) } ?? "Unknown"
        info["Resumable"]?.stringValue = r.resumable ? "Yes" : "No: pausing starts it over"
        info["Date Added"]?.stringValue = Self.dates.string(from: Date(timeIntervalSince1970: TimeInterval(r.added)))
        info["Date Finished"]?.stringValue = r.finished.map {
            Self.dates.string(from: Date(timeIntervalSince1970: TimeInterval($0)))
        } ?? "—"

        let done = r.status.isFinished
        addConnection.isEnabled = !done && r.allowedConnections < 32
        removeConnection.isEnabled = !done && r.allowedConnections > 1
        optionConnections.isEnabled = !done
        optionConnections.selectItem(withTitle: "\(r.allowedConnections)")
        reloadKeepingSelection(connectionsTable)
        reloadKeepingSelection(sourcesTable)
        let selected = sourcesTable.selectedRow
        removeMirror.isEnabled = selected >= 0 && selected < d.sources.count && !d.sources[selected].isOriginal
    }

    private func reloadKeepingSelection(_ t: NSTableView) {
        let selection = t.selectedRowIndexes
        t.reloadData()
        t.selectRowIndexes(selection.filteredIndexSet { $0 < t.numberOfRows }, byExtendingSelection: false)
    }

    // MARK: tables

    func numberOfRows(in tableView: NSTableView) -> Int {
        tableView === connectionsTable ? detail?.connections.count ?? 0 : detail?.sources.count ?? 0
    }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard let d = detail, let id = tableColumn?.identifier.rawValue else { return nil }
        let text: String
        if tableView === connectionsTable {
            guard row < d.connections.count else { return nil }
            let c = d.connections[row]
            text = switch id {
            case "host": c.host
            case "range": c.rangeText
            case "downloaded": c.downloadedText
            case "speed": c.speedText
            default: c.state
            }
        } else {
            guard row < d.sources.count else { return nil }
            let s = d.sources[row]
            if id == "on" {
                let box = NSButton(checkboxWithTitle: "", target: self, action: #selector(sourceToggled(_:)))
                box.state = s.enabled ? .on : .off
                box.tag = row
                box.toolTip = s.enabled ? "Turn this source off; its connections move to the others" : "Turn this source on"
                box.setAccessibilityLabel("Use \(s.url)")
                return box
            }
            text = switch id {
            case "url": s.isOriginal ? "\(s.url) (original)" : s.url
            case "state": s.statusText
            case "active": s.active > 0 ? "\(s.active)" : ""
            case "speed": s.speedText
            default: s.bytesText
            }
        }
        let field = NSTextField(labelWithString: text)
        field.lineBreakMode = id == "url" ? .byTruncatingMiddle : .byTruncatingTail
        field.toolTip = text
        if text.hasPrefix("Failed") || text == "Different file" || text == "File changed" { field.textColor = .systemRed }
        return field
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        guard let d = detail else { return }
        let selected = sourcesTable.selectedRow
        removeMirror.isEnabled = selected >= 0 && selected < d.sources.count && !d.sources[selected].isOriginal
    }

    // MARK: actions

    @objc func toggleRunning(_ sender: Any?) {
        guard let r = detail?.row else { return }
        if r.status.isRunning {
            (NSApp.delegate as? AppDelegate)?.mainWindow?.pause(rows: [r])
        } else {
            AppModel.shared.resume([id])
        }
    }

    @objc func closeWindow(_ sender: Any?) { close() }

    @objc func addOne(_ sender: Any?) {
        guard let r = detail?.row else { return }
        AppModel.shared.setConnections(id, min(32, r.allowedConnections + 1))
    }

    @objc func removeOne(_ sender: Any?) {
        guard let r = detail?.row else { return }
        AppModel.shared.setConnections(id, max(1, r.allowedConnections - 1))
    }

    @objc func connectionsChosen(_ sender: NSPopUpButton) {
        if let n = UInt32(sender.titleOfSelectedItem ?? "") { AppModel.shared.setConnections(id, n) }
    }

    @objc func sourceToggled(_ sender: NSButton) {
        guard let d = detail, sender.tag < d.sources.count else { return }
        AppModel.shared.manager.setSourceEnabled(id: id, url: d.sources[sender.tag].url, enabled: sender.state == .on)
        AppModel.shared.refresh()
    }

    @objc func addMirror(_ sender: Any?) {
        guard let window else { return }
        AddMirrorSheet.ask(for: id, on: window)
    }

    @objc func removeSelectedMirror(_ sender: Any?) {
        guard let d = detail, sourcesTable.selectedRow >= 0, sourcesTable.selectedRow < d.sources.count else { return }
        let source = d.sources[sourcesTable.selectedRow]
        guard !source.isOriginal else { return }
        AppModel.shared.manager.removeSource(id: id, url: source.url)
        AppModel.shared.refresh()
    }

    func windowWillClose(_ notification: Notification) {
        onClose?()
        onClose = nil
    }
}

/// The segments bar: the whole file as a track, each range filled up to what's downloaded (accent while a
/// connection works on it, green once complete, gray when waiting); the gaps are still to fetch.
final class SegmentsBar: NSView {
    private var segments: [SegmentInfo] = []
    private var size: UInt64 = 0
    private var finished = false

    func update(segments: [SegmentInfo], size: UInt64, finished: Bool) {
        self.segments = segments
        self.size = size
        self.finished = finished
        let done = segments.reduce(UInt64(0)) { $0 + ($1.pos - $1.start) }
        setAccessibilityValue(size > 0 ? "\(Int(Double(done) / Double(size) * 100))% downloaded" : "")
        needsDisplay = true
    }

    override func draw(_ dirtyRect: NSRect) {
        let track = NSBezierPath(roundedRect: bounds, xRadius: 4, yRadius: 4)
        NSColor.quaternaryLabelColor.setFill()
        track.fill()
        track.addClip()
        guard size > 0 else { return }
        let scale = bounds.width / CGFloat(size)
        if finished {
            NSColor.systemGreen.setFill()
            bounds.fill()
            return
        }
        for s in segments where s.pos > s.start {
            let rect = NSRect(x: CGFloat(s.start) * scale, y: 0, width: max(1, CGFloat(s.pos - s.start) * scale),
                              height: bounds.height)
            (s.pos >= s.end ? NSColor.systemGreen : s.active ? NSColor.controlAccentColor : NSColor.systemGray).setFill()
            rect.fill()
        }
        // A thin line where each range starts, so the segments read as separate pieces.
        NSColor.windowBackgroundColor.withAlphaComponent(0.6).setFill()
        for s in segments.dropFirst() {
            NSRect(x: CGFloat(s.start) * scale, y: 0, width: 1, height: bounds.height).fill()
        }
    }
}

/// Add Mirror… (§7.1): one address per line, for a running (M2) or stopped (M3) download.
@MainActor
enum AddMirrorSheet {
    static func ask(for id: UInt64, on window: NSWindow) {
        let alert = NSAlert()
        alert.messageText = "Add Mirror"
        alert.informativeText = "Other servers with the same file, one address per line. A running download starts using them at once."
        let text = NSTextView(frame: NSRect(x: 0, y: 0, width: 380, height: 90))
        text.isRichText = false
        text.font = .systemFont(ofSize: NSFont.systemFontSize)
        text.isAutomaticQuoteSubstitutionEnabled = false
        text.setAccessibilityIdentifier("mirror-links")
        let scroll = NSScrollView(frame: text.frame)
        scroll.documentView = text
        scroll.hasVerticalScroller = true
        scroll.borderType = .bezelBorder
        alert.accessoryView = scroll
        alert.addButton(withTitle: "Add")
        alert.addButton(withTitle: "Cancel")
        alert.window.initialFirstResponder = text
        alert.beginSheetModal(for: window) { response in
            MainActor.assumeIsolated {
                guard response == .alertFirstButtonReturn else { return }
                let links = findLinks(text: text.string)
                guard !links.isEmpty else { return }
                AppModel.shared.addMirrors(id, links)
                AppModel.shared.refresh()
            }
        }
    }
}
