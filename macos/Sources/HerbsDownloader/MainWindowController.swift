import AppKit
import HDCore

/// The main window (SPECIFICATION.md §2, §3): title with the search field at the right, toolbar, resizable sidebar
/// of filters, the download list, and a footer with the selection, active downloads and total speed.
@MainActor
final class MainWindowController: NSWindowController, NSToolbarDelegate, NSToolbarItemValidation, NSSearchFieldDelegate {
    private let split = NSSplitViewController()
    private let sidebar = SidebarViewController()
    let list = DownloadListViewController()
    private let footer = NSTextField(labelWithString: "")
    private var searchItem: NSSearchToolbarItem?

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
               help: "Stop All: pause every running download (⌥⌘.)", action: #selector(stopAll(_:))),
        Button(id: "delete", label: "Delete", symbol: "trash",
               help: "Delete: remove the selected downloads from the list (⌘⌫)", action: #selector(deleteSelected(_:))),
        Button(id: "settings", label: "Settings", symbol: "gearshape",
               help: "Settings (⌘,)", action: #selector(showSettings(_:))),
    ]

    /// Separators group T2–T3, T4–T5, T6, T7 and T8 (§3).
    private static let layout: [String] = [
        "newDownload", "sep", "resume", "pause", "sep", "startQueue", "stopQueue", "sep", "stopAll", "sep",
        "delete", "sep", "settings", NSToolbarItem.Identifier.flexibleSpace.rawValue, "search",
    ]

    init() {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1100, height: 640),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        window.title = "Herb's Downloader"
        window.minSize = NSSize(width: 760, height: 420)
        window.toolbarStyle = .unified
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

        let content = NSView()
        let splitView = split.view
        for v in [splitView, footer] {
            v.translatesAutoresizingMaskIntoConstraints = false
            content.addSubview(v)
        }
        NSLayoutConstraint.activate([
            splitView.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            splitView.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            splitView.topAnchor.constraint(equalTo: content.topAnchor),
            splitView.bottomAnchor.constraint(equalTo: footer.topAnchor, constant: -5),
            footer.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 12),
            footer.trailingAnchor.constraint(lessThanOrEqualTo: content.trailingAnchor, constant: -12),
            footer.bottomAnchor.constraint(equalTo: content.bottomAnchor, constant: -5),
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
        if id.rawValue == "search" {
            let item = NSSearchToolbarItem(itemIdentifier: id)
            item.searchField.placeholderString = "Search"
            item.searchField.delegate = self
            item.searchField.setAccessibilityIdentifier("search")
            item.searchField.toolTip = "Search: show downloads whose name or link contains the text (⌘F)"
            item.preferredWidthForSearchField = 220
            searchItem = item
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

    private func validate(_ action: Selector?) -> Bool {
        let selected = list.selectedRows
        switch action {
        case #selector(resumeSelected(_:)): return selected.contains { $0.status.canResume }
        case #selector(pauseSelected(_:)): return selected.contains { $0.status.isRunning }
        case #selector(stopAll(_:)): return AppModel.shared.rows.contains { $0.status.isRunning }
        case #selector(deleteSelected(_:)): return !selected.isEmpty
        // New Download (§5), the queue (§11) and Settings (§9) arrive with their own windows.
        case #selector(newDownload(_:)), #selector(startQueue(_:)), #selector(stopQueue(_:)), #selector(showSettings(_:)):
            return false
        default: return true
        }
    }

    // MARK: search

    func controlTextDidChange(_ notification: Notification) {
        guard let field = notification.object as? NSSearchField else { return }
        list.search = field.stringValue
    }

    @objc func focusSearch(_ sender: Any?) {
        guard let field = searchItem?.searchField else { return }
        window?.makeFirstResponder(field)
    }

    // MARK: actions

    @objc func newDownload(_ sender: Any?) {}
    @objc func startQueue(_ sender: Any?) {}
    @objc func stopQueue(_ sender: Any?) {}
    @objc func showSettings(_ sender: Any?) {}

    @objc func resumeSelected(_ sender: Any?) {
        AppModel.shared.resume(list.selectedRows.filter { $0.status.canResume }.map(\.id))
    }

    @objc func pauseSelected(_ sender: Any?) {
        AppModel.shared.pause(list.selectedRows.filter { $0.status.isRunning }.map(\.id))
    }

    @objc func stopAll(_ sender: Any?) {
        AppModel.shared.pauseAll()
    }

    /// Delete asks whether to remove only the list entry (the default) or the file as well (§10).
    @objc func deleteSelected(_ sender: Any?) {
        let rows = list.selectedRows
        guard !rows.isEmpty, let window else { return }
        let alert = NSAlert()
        alert.messageText = rows.count == 1 ? "Delete “\(rows[0].name)”?" : "Delete \(rows.count) downloads?"
        alert.informativeText = "Remove them from the list only, or delete their files as well?"
        alert.addButton(withTitle: "Remove from List")
        alert.addButton(withTitle: "Cancel")
        alert.addButton(withTitle: "Delete Files Too")
        alert.buttons[2].hasDestructiveAction = true
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
