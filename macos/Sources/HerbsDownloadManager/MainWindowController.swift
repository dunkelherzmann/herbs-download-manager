import AppKit
import HDCore

/// The main window (SPECIFICATION.md §2, §3): title with the search field at the right, toolbar, resizable sidebar
/// of filters, the download list, and a footer with the selection, active downloads and total speed.
@MainActor
final class MainWindowController: NSWindowController, NSToolbarDelegate, NSToolbarItemValidation, NSMenuItemValidation {
    private let split = NSSplitViewController()
    private let sidebar = SidebarViewController()
    let list = DownloadListViewController()
    private let footer = NSTextField(labelWithString: "")
    private let findBar = FindBar()
    private weak var keepOnTopItem: NSToolbarItem?
    /// Open New Download, Add Links and Batch windows (kept alive while shown).
    private var panels: [NSWindowController] = []
    /// Open Download windows by download id: one each.
    private var downloadWindows: [UInt64: DownloadWindowController] = [:]

    /// T1–T8 with their symbols, hover text and shortcut (shown in the hover text).
    private struct Button {
        let id: String
        let label: String
        let symbol: String
        let help: String
        let action: Selector
    }

    private static let buttons: [Button] = [
        Button(id: "newDownload", label: "New Download", symbol: "link.badge.plus",
               help: "New Download: add a link to download (⌘N)", action: #selector(newDownload(_:))),
        Button(id: "resume", label: "Resume", symbol: "play.fill",
               help: "Resume: continue the selected downloads (⌘R)", action: #selector(resumeSelected(_:))),
        Button(id: "pause", label: "Pause", symbol: "pause.fill",
               help: "Pause: stop the selected downloads, keeping what's downloaded (⌘.)", action: #selector(pauseSelected(_:))),
        Button(id: "startQueue", label: "Start Queue", symbol: "text.line.first.and.arrowtriangle.forward",
               help: "Start Queue: download waiting items one after another (⇧⌘R)", action: #selector(startQueue(_:))),
        Button(id: "stopQueue", label: "Stop Queue", symbol: "stop.circle",
               help: "Stop Queue: stop starting new items from the queue", action: #selector(stopQueue(_:))),
        Button(id: "stopAll", label: "Stop All", symbol: "stop.fill",
               help: "Stop All: stop the queue and pause every running download (⌥⌘.)", action: #selector(stopAll(_:))),
        Button(id: "delete", label: "Delete", symbol: "trash",
               help: "Delete: remove the selected downloads from the list (⌘⌫)", action: #selector(deleteSelected(_:))),
        Button(id: "settings", label: "Settings", symbol: "gearshape",
               help: "Settings (⌘,)", action: #selector(showSettings(_:))),
        Button(id: "search", label: "Search", symbol: "magnifyingglass",
               help: "Search: show only downloads whose name or link contains some text (⌘F)",
               action: #selector(toggleSearch(_:))),
    ]

    /// Separators group T2–T3, T4–T5, T6, T7 and T8 (§3).
    private static let layout: [String] = [
        "newDownload", "sep", "resume", "pause", "sep", "startQueue", "stopQueue", "sep", "stopAll", "sep",
        "delete", "sep", "settings", NSToolbarItem.Identifier.flexibleSpace.rawValue, "keepOnTop", "search", "mainMenu",
    ]

    init() {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1100, height: 640),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "Herb's Download Manager"
        window.minSize = NSSize(width: 760, height: 420)
        window.toolbarStyle = .unified
        // The title bar is one solid band across the window with a clear line under it; the sidebar and its divider
        // start below that line. A sidebar running up under the title bar (full-size content) let macOS blend its
        // edge into the title bar, which looked unfinished.
        window.titlebarSeparatorStyle = .line
        window.setAccessibilityIdentifier("main-window")
        super.init(window: window)

        let toolbar = NSToolbar(identifier: "MainToolbar")
        toolbar.delegate = self
        toolbar.displayMode = .iconOnly
        toolbar.allowsUserCustomization = false
        window.toolbar = toolbar

        let sidebarItem = NSSplitViewItem(sidebarWithViewController: sidebar)
        sidebarItem.minimumThickness = 190
        sidebarItem.maximumThickness = 360
        sidebarItem.canCollapse = true
        let listItem = NSSplitViewItem(viewController: list)
        split.addSplitViewItem(sidebarItem)
        split.addSplitViewItem(listItem)
        split.splitView.autosaveName = "MainSplit"

        footer.font = .monospacedDigitSystemFont(ofSize: NSFont.smallSystemFontSize, weight: .regular)
        footer.textColor = .secondaryLabelColor
        footer.setAccessibilityIdentifier("footer")

        let content = DropView()
        content.onLinks = { [weak self] links in self?.openLinks(links) }
        let splitView = split.view
        // The footer is a bar of its own: a hairline above it and the window's bar background, as in Finder's and
        // Mail's status bars, so the list visibly ends where it starts.
        let footerBar = NSVisualEffectView()
        footerBar.material = .titlebar
        footerBar.blendingMode = .withinWindow
        let footerLine = NSBox()
        footerLine.boxType = .separator
        findBar.onChange = { [weak self] text in self?.list.search = text }
        for v in [splitView, footerBar, footerLine, findBar] {
            v.translatesAutoresizingMaskIntoConstraints = false
            content.addSubview(v)
        }
        footer.translatesAutoresizingMaskIntoConstraints = false
        footerBar.addSubview(footer)
        NSLayoutConstraint.activate([
            splitView.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            splitView.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            splitView.topAnchor.constraint(equalTo: content.topAnchor),
            splitView.bottomAnchor.constraint(equalTo: footerLine.topAnchor),
            footerLine.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            footerLine.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            footerLine.bottomAnchor.constraint(equalTo: footerBar.topAnchor),
            footerLine.heightAnchor.constraint(equalToConstant: 1),
            footerBar.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            footerBar.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            footerBar.bottomAnchor.constraint(equalTo: content.bottomAnchor),
            footerBar.heightAnchor.constraint(equalToConstant: 26),
            footer.leadingAnchor.constraint(equalTo: footerBar.leadingAnchor, constant: 12),
            footer.trailingAnchor.constraint(lessThanOrEqualTo: footerBar.trailingAnchor, constant: -12),
            footer.centerYAnchor.constraint(equalTo: footerBar.centerYAnchor),
            // Top right of the list, just below its column titles, as Herb's Terminal's find bar.
            findBar.topAnchor.constraint(equalTo: content.topAnchor, constant: 34),
            findBar.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -12),
        ])
        let host = NSViewController()
        host.view = content
        host.addChild(split)
        window.contentViewController = host
        window.setContentSize(NSSize(width: 1100, height: 640))
        window.setFrameAutosaveName("MainWindow")
        if !window.setFrameUsingName("MainWindow") { window.center() }

        sidebar.onSelect = { [weak self] filter, kind in
            self?.list.filter = filter
            self?.list.kind = kind
        }
        list.onSelectionChange = { [weak self] in self?.update() }
        list.onLinks = { [weak self] links in self?.openLinks(links) }
        AppModel.shared.observe { [weak self] in self?.update() }
        update()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    /// Footer text and button states.
    private func update() {
        let rows = AppModel.shared.rows
        let active = rows.filter { $0.status.isRunning }
        let speed = active.reduce(UInt64(0)) { $0 + $1.speed }
        let selected = list.selectedRows.count
        var parts = ["\(selected) selected", "\(active.count) active"]
        if speed > 0 { parts.append(speedText(bytesPerSecond: speed)) }
        let waiting = rows.filter { $0.status == .waiting }.count
        if AppModel.shared.queueRunning {
            parts.append(waiting == 0 ? "Queue running" : "Queue running, \(waiting) waiting")
        } else if waiting > 0 {
            parts.append("Queue stopped, \(waiting) waiting")
        }
        if let notice = AppModel.shared.notice { parts.append(notice) }
        footer.stringValue = parts.joined(separator: "  ·  ")
        window?.toolbar?.validateVisibleItems()
    }

    // MARK: toolbar

    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        Self.layout.enumerated().map { i, id in id == "sep" ? .init("sep\(i)") : .init(id) }
    }

    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        toolbarDefaultItemIdentifiers(toolbar)
    }

    func toolbar(_ toolbar: NSToolbar, itemForItemIdentifier id: NSToolbarItem.Identifier,
                 willBeInsertedIntoToolbar flag: Bool) -> NSToolbarItem? {
        if id.rawValue.hasPrefix("sep") {
            // A thin vertical line between groups.
            let item = NSToolbarItem(itemIdentifier: id)
            let line = NSBox()
            line.boxType = .separator
            line.translatesAutoresizingMaskIntoConstraints = false
            line.widthAnchor.constraint(equalToConstant: 1).isActive = true
            line.heightAnchor.constraint(equalToConstant: 18).isActive = true
            item.view = line
            return item
        }
        if id.rawValue == "mainMenu" { return InWindowMenu.toolbarItem(id) }
        if id.rawValue == "keepOnTop" {
            let item = NSToolbarItem(itemIdentifier: id)
            item.label = "Keep on Top"
            item.paletteLabel = "Keep on Top"
            item.isBordered = true
            item.target = self
            item.action = #selector(toggleKeepOnTop(_:))
            item.autovalidates = true
            keepOnTopItem = item
            showKeepOnTop()
            return item
        }
        guard let b = Self.buttons.first(where: { $0.id == id.rawValue }) else { return nil }
        let item = NSToolbarItem(itemIdentifier: id)
        item.label = b.label
        item.paletteLabel = b.label
        item.toolTip = b.help
        item.image = NSImage(systemSymbolName: b.symbol, accessibilityDescription: b.label)
        item.isBordered = true
        item.target = self
        item.action = b.action
        item.autovalidates = true
        return item
    }

    func validateToolbarItem(_ item: NSToolbarItem) -> Bool {
        validate(item.action)
    }

    func validateMenuItem(_ item: NSMenuItem) -> Bool {
        if item.action == #selector(toggleKeepOnTop(_:)) { item.state = KeepOnTop.isOn ? .on : .off }
        if item.action == #selector(toggleSidebar(_:)) {
            let collapsed = split.splitViewItems.first?.isCollapsed ?? false
            item.title = collapsed ? "Show Sidebar" : "Hide Sidebar"
        }
        return validate(item.action)
    }

    /// The downloads a command acts on: the right-clicked one (with the selection when it is part of it), else
    /// the selection.
    var targets: [DownloadRow] { list.selectedRows }

    private func validate(_ action: Selector?) -> Bool {
        let selected = targets
        let rows = AppModel.shared.rows
        switch action {
        case #selector(resumeSelected(_:)): return selected.contains { $0.status.canResume }
        case #selector(pauseSelected(_:)): return selected.contains { $0.status.isRunning }
        case #selector(restartSelected(_:)): return !selected.isEmpty
        case #selector(stopAll(_:)): return rows.contains { $0.status.isRunning } || AppModel.shared.queueRunning
        case #selector(startQueue(_:)): return !AppModel.shared.queueRunning
        case #selector(stopQueue(_:)): return AppModel.shared.queueRunning
        case #selector(deleteSelected(_:)): return !selected.isEmpty
        case #selector(deleteAllFinished(_:)): return rows.contains { $0.status.isFinished }
        case #selector(deleteAllUnfinished(_:)): return rows.contains { !$0.status.isFinished }
        case #selector(deleteMissingFiles(_:)): return !Self.missing(rows).isEmpty
        case #selector(deleteEntireList(_:)): return !rows.isEmpty
        case #selector(openSelected(_:)), #selector(showInFinder(_:)):
            return selected.count == 1 && selected[0].path.map { FileManager.default.fileExists(atPath: $0) } == true
        case #selector(copyLink(_:)): return !selected.isEmpty
        case #selector(addConnection(_:)):
            return selected.count == 1 && !selected[0].status.isFinished && selected[0].allowedConnections < 32
        case #selector(removeConnection(_:)):
            return selected.count == 1 && !selected[0].status.isFinished && selected[0].allowedConnections > 1
        case #selector(toggleSidebar(_:)), #selector(focusSearch(_:)), #selector(toggleSearch(_:)),
             #selector(toggleKeepOnTop(_:)): return true
        case #selector(newDownload(_:)), #selector(addFromClipboard(_:)), #selector(batchDownload(_:)),
             #selector(importLinks(_:)): return true
        case #selector(showDownloadWindow(_:)), #selector(showProperties(_:)): return !selected.isEmpty
        case #selector(addMirror(_:)): return selected.count == 1
        default: return true
        }
    }

    /// Finished downloads whose file is no longer on disk.
    private static func missing(_ rows: [DownloadRow]) -> [DownloadRow] {
        rows.filter { $0.status.isFinished && !($0.path.map { FileManager.default.fileExists(atPath: $0) } ?? false) }
    }

    // MARK: search

    /// Find (⌘F): opens the find bar, or moves the cursor back into it.
    @objc func focusSearch(_ sender: Any?) { findBar.open() }

    /// Keep on Top (T11): every window of the app above other apps' windows, or back to normal.
    @objc func toggleKeepOnTop(_ sender: Any?) {
        KeepOnTop.isOn.toggle()
        showKeepOnTop()
    }

    /// The pin is filled and tinted while Keep on Top is on; its hover text and VoiceOver value say which.
    private func showKeepOnTop() {
        guard let item = keepOnTopItem else { return }
        let on = KeepOnTop.isOn
        let image = NSImage(systemSymbolName: on ? "pin.fill" : "pin", accessibilityDescription: "Keep on Top")
        item.image = on ? image?.withSymbolConfiguration(.init(paletteColors: [.controlAccentColor])) : image
        item.toolTip = on ? "Keep on Top is on: Herb's DM stays above other apps' windows. Click to turn it off (⌃⌘T)"
                          : "Keep on Top: keep Herb's DM above every other app's windows (⌃⌘T)"
    }

    /// The toolbar's 🔍: opens or closes the find bar.
    @objc func toggleSearch(_ sender: Any?) { findBar.toggle() }

    // MARK: actions

    /// New Download (§5.1), its Link filled from the clipboard when it holds a link.
    @objc func newDownload(_ sender: Any?) {
        let clip = NSPasteboard.general.string(forType: .string).map { findLinks(text: $0) } ?? []
        show(NewDownloadWindowController(link: clip.first, onClose: closer()))
    }

    /// Add from Clipboard (§5.2): every link in the clipboard's text opens Add Links.
    @objc func addFromClipboard(_ sender: Any?) {
        let pasteboard = NSPasteboard.general
        var text = pasteboard.string(forType: .string) ?? ""
        if let html = pasteboard.string(forType: .html) { text += "\n" + html }
        let links = findLinks(text: text)
        guard !links.isEmpty, let window else {
            let alert = NSAlert()
            alert.messageText = "The clipboard has no links."
            alert.informativeText = "Copy one or more web addresses (http, https or ftp), then try again."
            if let window = self.window { alert.beginSheetModal(for: window) } else { alert.runModal() }
            return
        }
        _ = window
        openLinks(links)
    }

    /// Batch Download (§5.5).
    @objc func batchDownload(_ sender: Any?) {
        show(BatchWindowController(onLinks: { [weak self] links in self?.openLinks(links) }, onClose: closer()))
    }

    /// Add Links (§5.3) for links from the clipboard, a drop, the Dock icon or a batch.
    /// File ▸ Import Links… (⇧⌘O, §5.7): links from text-based files (with no extension, .txt, .md, .rtf, .html, …)
    /// open the Add Links window, where links to the same file are added as one download and its mirrors.
    @objc func importLinks(_ sender: Any?) {
        guard let window else { return }
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        panel.message = "Choose files with links: plain text with or without an extension, Markdown, rich text, HTML…"
        panel.prompt = "Import"
        panel.beginSheetModal(for: window) { response in
            MainActor.assumeIsolated {
                guard response == .OK else { return }
                self.importLinks(from: panel.urls)
            }
        }
    }

    func importLinks(from files: [URL]) {
        var links: [String] = []
        for file in files {
            for link in LinkReader.links(fromFile: file) where !links.contains(link) { links.append(link) }
        }
        guard !links.isEmpty else {
            let alert = NSAlert()
            alert.messageText = files.count == 1 ? "“\(files[0].lastPathComponent)” has no links."
                : "These files have no links."
            alert.informativeText = "Import Links reads text files (with or without an extension), Markdown, rich text "
                + "and HTML, and finds the web addresses (http, https or ftp) in them."
            if let window { alert.beginSheetModal(for: window) } else { alert.runModal() }
            return
        }
        openLinks(links)
    }

    /// Opens Add Links with the links, or, when it is already open, adds them to it as more rows.
    func openLinks(_ links: [String]) {
        guard !links.isEmpty else { return }
        showWindow(nil)
        if let open = panels.last(where: { $0 is AddLinksWindowController && $0.window?.isVisible == true })
            as? AddLinksWindowController {
            open.addLinks(links)
            open.window?.makeKeyAndOrderFront(nil)
            return
        }
        show(AddLinksWindowController(links: links, onClose: closer()))
    }

    private func show(_ panel: NSWindowController) {
        panels.append(panel)
        panel.showWindow(nil)
        panel.window?.makeKeyAndOrderFront(nil)
    }

    /// Forgets a panel once it closes.
    private func closer() -> () -> Void {
        let count = panels.count
        return { [weak self] in
            guard let self else { return }
            if count < self.panels.count { self.panels.remove(at: count) } else { self.panels.removeAll { $0.window?.isVisible == false } }
        }
    }
    /// Start Queue (T4, ⇧⌘R, §11): waiting downloads start, oldest first, a few at a time.
    @objc func startQueue(_ sender: Any?) { AppModel.shared.startQueue() }
    /// Stop Queue (T5): no more waiting downloads start; running ones carry on.
    @objc func stopQueue(_ sender: Any?) { AppModel.shared.stopQueue() }
    @objc func showSettings(_ sender: Any?) { SettingsWindowController.shared.show() }
    /// Add Mirror… (⌥⌘M, §7.1) on the selected download.
    @objc func addMirror(_ sender: Any?) {
        guard let row = targets.first, targets.count == 1, let window else { return }
        AddMirrorSheet.ask(for: row.id, on: window)
    }

    /// Properties (⌘I): the Download window's Info tab.
    @objc func showProperties(_ sender: Any?) {
        for row in targets { openDownloadWindow(row.id)?.showTab("Info") }
    }

    /// Show Download Window (⌘O, §6).
    @objc func showDownloadWindow(_ sender: Any?) {
        for row in targets { openDownloadWindow(row.id) }
    }

    @discardableResult
    func openDownloadWindow(_ id: UInt64) -> DownloadWindowController? {
        if let open = downloadWindows[id] {
            open.showWindow(nil)
            return open
        }
        let controller = DownloadWindowController(id: id, onClose: { [weak self] in self?.downloadWindows[id] = nil })
        downloadWindows[id] = controller
        controller.showWindow(nil)
        return controller
    }

    /// Pause from a Download window: the same question as the toolbar's Pause.
    func pause(rows: [DownloadRow]) {
        let running = rows.filter { $0.status.isRunning }
        confirmPause(running) { AppModel.shared.pause(running.map(\.id)) }
    }

    @objc func resumeSelected(_ sender: Any?) {
        AppModel.shared.resume(targets.filter { $0.status.canResume }.map(\.id))
    }

    @objc func pauseSelected(_ sender: Any?) {
        let rows = targets.filter { $0.status.isRunning }
        confirmPause(rows) { AppModel.shared.pause(rows.map(\.id)) }
    }

    @objc func stopAll(_ sender: Any?) {
        confirmPause(AppModel.shared.rows.filter { $0.status.isRunning }) { AppModel.shared.pauseAll() }
    }

    /// Pausing a download no source can resume asks first, Cancel the default (§10).
    private func confirmPause(_ rows: [DownloadRow], then pause: @escaping @MainActor () -> Void) {
        let lost = rows.filter { !$0.resumable }
        guard !lost.isEmpty, let window else { return pause() }
        let alert = NSAlert()
        alert.messageText = lost.count == 1 && rows.count == 1
            ? "This download can't be resumed and will start over. Pause anyway?"
            : "\(lost.count == 1 ? "“\(lost[0].name)”" : "\(lost.count) of these downloads") can't be resumed and will start over. Pause anyway?"
        alert.addButton(withTitle: "Cancel")
        alert.addButton(withTitle: "Pause")
        alert.beginSheetModal(for: window) { response in
            MainActor.assumeIsolated {
                if response == .alertSecondButtonReturn { pause() }
            }
        }
    }

    @objc func toggleSidebar(_ sender: Any?) {
        split.toggleSidebar(sender)
    }

    /// Restart discards the downloaded data after confirming, Cancel the default (§10).
    @objc func restartSelected(_ sender: Any?) {
        let rows = targets
        guard !rows.isEmpty, let window else { return }
        let alert = NSAlert()
        alert.messageText = rows.count == 1 ? "Restart “\(rows[0].name)”?" : "Restart \(rows.count) downloads?"
        alert.informativeText = "What was downloaded is discarded and the download starts again from the beginning."
        alert.addButton(withTitle: "Cancel")
        alert.addButton(withTitle: "Restart")
        alert.buttons[1].hasDestructiveAction = true
        alert.beginSheetModal(for: window) { response in
            MainActor.assumeIsolated {
                if response == .alertSecondButtonReturn { AppModel.shared.restart(rows.map(\.id)) }
            }
        }
    }

    @objc func openSelected(_ sender: Any?) {
        guard let path = targets.first?.path else { return }
        NSWorkspace.shared.open(URL(fileURLWithPath: path))
    }

    /// Open With ▸ an app: the menu item carries the app's URL.
    @objc func openWith(_ sender: NSMenuItem) {
        guard let path = targets.first?.path, let app = sender.representedObject as? URL else { return }
        NSWorkspace.shared.open([URL(fileURLWithPath: path)], withApplicationAt: app,
                                configuration: NSWorkspace.OpenConfiguration())
    }

    @objc func showInFinder(_ sender: Any?) {
        guard let path = targets.first?.path else { return }
        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)])
    }

    @objc func copyLink(_ sender: Any?) {
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(targets.map(\.url).joined(separator: "\n"), forType: .string)
    }

    @objc func addConnection(_ sender: Any?) {
        guard let row = targets.first else { return }
        AppModel.shared.setConnections(row.id, row.allowedConnections + 1)
    }

    @objc func removeConnection(_ sender: Any?) {
        guard let row = targets.first else { return }
        AppModel.shared.setConnections(row.id, row.allowedConnections - 1)
    }

    @objc func deleteSelected(_ sender: Any?) { confirmDelete(targets) }
    @objc func deleteAllFinished(_ sender: Any?) { confirmDelete(AppModel.shared.rows.filter { $0.status.isFinished }) }
    @objc func deleteAllUnfinished(_ sender: Any?) { confirmDelete(AppModel.shared.rows.filter { !$0.status.isFinished }) }
    @objc func deleteMissingFiles(_ sender: Any?) { confirmDelete(Self.missing(AppModel.shared.rows), filesGone: true) }
    @objc func deleteEntireList(_ sender: Any?) { confirmDelete(AppModel.shared.rows) }

    /// Delete asks whether to remove only the list entries (the default) or the files as well (§10).
    private func confirmDelete(_ rows: [DownloadRow], filesGone: Bool = false) {
        guard !rows.isEmpty, let window else { return }
        let alert = NSAlert()
        alert.messageText = rows.count == 1 ? "Delete “\(rows[0].name)”?" : "Delete \(rows.count) downloads?"
        alert.addButton(withTitle: "Remove from List")
        alert.addButton(withTitle: "Cancel")
        if filesGone {
            alert.informativeText = "Their files are no longer on disk; only the list entries are removed."
        } else {
            alert.informativeText = "Remove them from the list only, or delete their files as well?"
            alert.addButton(withTitle: "Delete Files Too")
            alert.buttons[2].hasDestructiveAction = true
        }
        alert.beginSheetModal(for: window) { response in
            MainActor.assumeIsolated {
                switch response {
                case .alertFirstButtonReturn: AppModel.shared.remove(rows.map(\.id), deleteFiles: false)
                case .alertThirdButtonReturn: AppModel.shared.remove(rows.map(\.id), deleteFiles: true)
                default: break
                }
            }
        }
    }
}

/// The main window's content: dropping links, text or .txt/.html files adds every link found (§5.4); while
/// dragging, the window dims and shows "Drop links here".
final class DropView: NSView {
    var onLinks: (([String]) -> Void)?
    private let overlay = NSView()
    let message = NSTextField(labelWithString: "Drop links here")

    override init(frame: NSRect) {
        super.init(frame: frame)
        registerForDraggedTypes([.string, .URL, .fileURL, .html])
        overlay.wantsLayer = true
        overlay.layer?.backgroundColor = NSColor.black.withAlphaComponent(0.45).cgColor
        overlay.isHidden = true
        message.font = .systemFont(ofSize: 24, weight: .semibold)
        message.textColor = .white
        message.translatesAutoresizingMaskIntoConstraints = false
        overlay.addSubview(message)
        NSLayoutConstraint.activate([
            message.centerXAnchor.constraint(equalTo: overlay.centerXAnchor),
            message.centerYAnchor.constraint(equalTo: overlay.centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    override func layout() {
        super.layout()
        overlay.frame = bounds
        if overlay.superview == nil || subviews.last !== overlay {
            overlay.removeFromSuperview()
            addSubview(overlay)
        }
    }

    static func links(from pasteboard: NSPasteboard) -> [String] {
        var text = ""
        if let urls = pasteboard.readObjects(forClasses: [NSURL.self], options: nil) as? [URL] {
            for url in urls {
                if url.isFileURL {
                    text += "\n" + (LinkReader.text(ofFile: url) ?? "")
                } else {
                    text += "\n" + url.absoluteString
                }
            }
        }
        text += "\n" + (pasteboard.string(forType: .string) ?? "")
        text += "\n" + (pasteboard.string(forType: .html) ?? "")
        return findLinks(text: text)
    }

    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        guard !Self.links(from: sender.draggingPasteboard).isEmpty else { return [] }
        overlay.isHidden = false
        return .copy
    }

    override func draggingExited(_ sender: NSDraggingInfo?) { overlay.isHidden = true }

    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        overlay.isHidden = true
        let links = Self.links(from: sender.draggingPasteboard)
        guard !links.isEmpty else { return false }
        DispatchQueue.main.async { self.onLinks?(links) }
        return true
    }
}
