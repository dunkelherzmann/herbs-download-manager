import AppKit
import HDCore
import UniformTypeIdentifiers

/// The download list (SPECIFICATION.md §2.2): sortable columns L1–L10, every one but Name can be hidden from the
/// header's context menu; newest first by default.
@MainActor
final class DownloadListViewController: NSViewController, NSTableViewDataSource, NSTableViewDelegate, NSMenuDelegate {
    struct Column {
        let id: String
        let title: String
        let width: CGFloat
        let hiddenByDefault: Bool
    }

    static let columns: [Column] = [
        Column(id: "name", title: "Name", width: 280, hiddenByDefault: false),
        Column(id: "note", title: "Note", width: 180, hiddenByDefault: false),
        Column(id: "status", title: "Status", width: 150, hiddenByDefault: false),
        Column(id: "size", title: "Size", width: 140, hiddenByDefault: false),
        Column(id: "speed", title: "Speed", width: 90, hiddenByDefault: false),
        Column(id: "timeLeft", title: "Time Left", width: 90, hiddenByDefault: false),
        Column(id: "connections", title: "Connections", width: 90, hiddenByDefault: false),
        Column(id: "mirrors", title: "Mirrors", width: 70, hiddenByDefault: false),
        Column(id: "added", title: "Date Added", width: 140, hiddenByDefault: false),
        Column(id: "finished", title: "Date Finished", width: 140, hiddenByDefault: true),
        Column(id: "location", title: "Location", width: 200, hiddenByDefault: true),
    ]

    let table = NSTableView()
    var filter: Filter = .all { didSet { reload() } }
    var kind: FileKind? { didSet { reload() } }
    var search = "" { didSet { reload() } }
    var onSelectionChange: (() -> Void)?
    /// Links dropped on the list but not on an unfinished download: they open Add Links.
    var onLinks: (([String]) -> Void)?
    private(set) var shown: [DownloadRow] = []
    private let empty = NSTextField(labelWithString: "No downloads yet")
    private let contextMenu = NSMenu(title: "Download")

    private static let dateFormat: DateFormatter = {
        let f = DateFormatter()
        f.dateStyle = .medium
        f.timeStyle = .short
        f.doesRelativeDateFormatting = true
        return f
    }()

    var selectedRows: [DownloadRow] {
        table.selectedRowIndexes.compactMap { $0 < shown.count ? shown[$0] : nil }
    }

    /// Selects these downloads (the feature tests' way to point a command at them).
    func select(_ ids: [UInt64]) {
        let rows = IndexSet(shown.indices.filter { ids.contains(shown[$0].id) })
        table.selectRowIndexes(rows, byExtendingSelection: false)
    }

    override func loadView() {
        for c in Self.columns {
            let column = NSTableColumn(identifier: .init(c.id))
            column.title = c.title
            column.width = c.width
            column.minWidth = 50
            column.isHidden = c.hiddenByDefault
            column.sortDescriptorPrototype = NSSortDescriptor(key: c.id, ascending: c.id != "added")
            column.headerToolTip = c.title
            table.addTableColumn(column)
        }
        table.usesAlternatingRowBackgroundColors = true
        table.allowsMultipleSelection = true
        table.allowsColumnReordering = true
        table.columnAutoresizingStyle = .lastColumnOnlyAutoresizingStyle
        table.rowHeight = 30
        table.style = .fullWidth
        table.dataSource = self
        table.delegate = self
        table.target = self
        table.doubleAction = #selector(openRow(_:))
        table.sortDescriptors = [NSSortDescriptor(key: "added", ascending: false)]
        table.autosaveName = TestCopy.autosave("DownloadList")
        table.autosaveTableColumns = true
        // A layout saved before the Note column existed puts it last, past the window's edge: place it after Name once.
        let placed = TestCopy.autosave("NoteColumnPlaced")
        if !UserDefaults.standard.bool(forKey: placed), let from = table.tableColumns.firstIndex(where: { $0.identifier.rawValue == "note" }) {
            table.moveColumn(from, toColumn: 1)
            UserDefaults.standard.set(true, forKey: placed)
        }
        table.setAccessibilityIdentifier("downloads")
        table.setAccessibilityLabel("Downloads")
        table.registerForDraggedTypes([.string, .URL, .fileURL, .html])
        let headerMenu = NSMenu()
        headerMenu.delegate = self
        table.headerView?.menu = headerMenu
        contextMenu.delegate = self
        table.menu = contextMenu

        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.hasHorizontalScroller = true

        empty.font = .systemFont(ofSize: 17, weight: .medium)
        empty.textColor = .secondaryLabelColor
        empty.translatesAutoresizingMaskIntoConstraints = false
        empty.setAccessibilityIdentifier("empty-list")

        let root = NSView()
        scroll.translatesAutoresizingMaskIntoConstraints = false
        root.addSubview(scroll)
        root.addSubview(empty)
        NSLayoutConstraint.activate([
            scroll.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            scroll.topAnchor.constraint(equalTo: root.topAnchor),
            scroll.bottomAnchor.constraint(equalTo: root.bottomAnchor),
            empty.centerXAnchor.constraint(equalTo: root.centerXAnchor),
            empty.centerYAnchor.constraint(equalTo: root.centerYAnchor),
        ])
        view = root
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        AppModel.shared.observe { [weak self] in self?.reload() }
        reload()
    }

    // MARK: rows

    func reload() {
        let selected = Set(selectedRows.map(\.id))
        var rows = AppModel.shared.rows.filter { row in
            filterMatches(filter: filter, status: row.status)
                && (kind == nil || row.kind == kind)
                && searchMatches(row: row, text: search)
        }
        if let sort = table.sortDescriptors.first, let key = sort.key {
            rows.sort { a, b in
                let order = Self.compare(a, b, key)
                return order == .orderedSame ? a.id > b.id : (order == .orderedAscending) == sort.ascending
            }
        }
        let sameOrder = rows.map(\.id) == shown.map(\.id)
        shown = rows
        if sameOrder {
            let visible = table.rows(in: table.visibleRect)
            if visible.length > 0 {
                table.reloadData(forRowIndexes: IndexSet(integersIn: visible.location..<NSMaxRange(visible)),
                                 columnIndexes: IndexSet(integersIn: 0..<table.numberOfColumns))
            }
        } else {
            table.reloadData()
            let keep = IndexSet(shown.indices.filter { selected.contains(shown[$0].id) })
            table.selectRowIndexes(keep, byExtendingSelection: false)
        }
        empty.isHidden = !shown.isEmpty
        empty.stringValue = AppModel.shared.rows.isEmpty ? "No downloads yet" : "No downloads match"
    }

    private static func compare(_ a: DownloadRow, _ b: DownloadRow, _ key: String) -> ComparisonResult {
        func cmp<T: Comparable>(_ x: T, _ y: T) -> ComparisonResult { x < y ? .orderedAscending : x > y ? .orderedDescending : .orderedSame }
        switch key {
        case "name": return a.name.localizedStandardCompare(b.name)
        case "status": return a.statusText.localizedStandardCompare(b.statusText)
        case "size": return cmp(a.size ?? 0, b.size ?? 0)
        case "speed": return cmp(a.speed, b.speed)
        case "timeLeft": return cmp(a.timeLeft ?? .max, b.timeLeft ?? .max)
        case "connections": return cmp(a.activeConnections, b.activeConnections)
        case "mirrors": return cmp(a.sourcesKnown, b.sourcesKnown)
        case "added": return cmp(a.added, b.added)
        case "finished": return cmp(a.finished ?? 0, b.finished ?? 0)
        case "location": return a.folder.localizedStandardCompare(b.folder)
        case "note": return a.note.localizedStandardCompare(b.note)
        default: return .orderedSame
        }
    }

    func numberOfRows(in tableView: NSTableView) -> Int { shown.count }

    // MARK: dropping links

    /// Links dropped on an unfinished download become its mirrors (§7.1); anywhere else on the list they open Add
    /// Links. The row or the whole list is highlighted to show which.
    func tableView(_ tableView: NSTableView, validateDrop info: NSDraggingInfo, proposedRow row: Int,
                   proposedDropOperation dropOperation: NSTableView.DropOperation) -> NSDragOperation {
        guard !DropView.links(from: info.draggingPasteboard).isEmpty else { return [] }
        if dropOperation == .on, row >= 0, row < shown.count, !shown[row].status.isFinished {
            return .copy
        }
        tableView.setDropRow(-1, dropOperation: .on)
        return .copy
    }

    func tableView(_ tableView: NSTableView, acceptDrop info: NSDraggingInfo, row: Int,
                   dropOperation: NSTableView.DropOperation) -> Bool {
        let links = DropView.links(from: info.draggingPasteboard)
        guard !links.isEmpty else { return false }
        if row >= 0, row < shown.count, !shown[row].status.isFinished {
            AppModel.shared.addMirrors(shown[row].id, links)
        } else {
            DispatchQueue.main.async { self.onLinks?(links) }
        }
        return true
    }

    func tableView(_ tableView: NSTableView, sortDescriptorsDidChange oldDescriptors: [NSSortDescriptor]) {
        shown = []
        reload()
    }

    func tableViewSelectionDidChange(_ notification: Notification) { onSelectionChange?() }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard let id = tableColumn?.identifier.rawValue, row < shown.count else { return nil }
        let item = shown[row]
        if id == "name" {
            let cell = (tableView.makeView(withIdentifier: NameCell.id, owner: nil) as? NameCell) ?? NameCell()
            cell.show(item)
            return cell
        }
        let ident = NSUserInterfaceItemIdentifier("text-\(id)")
        let cell = (tableView.makeView(withIdentifier: ident, owner: nil) as? NSTableCellView) ?? Self.textCell(ident)
        let text: String
        switch id {
        case "status": text = item.statusText
        case "size": text = item.sizeText
        case "speed": text = item.speedText
        case "timeLeft": text = item.timeLeftText
        case "connections": text = item.status.isFinished ? "" : "\(item.activeConnections) / \(item.allowedConnections)"
        case "mirrors": text = "\(item.sourcesInUse) / \(item.sourcesKnown)"
        case "added": text = Self.dateFormat.string(from: Date(timeIntervalSince1970: TimeInterval(item.added)))
        case "finished": text = item.finished.map { Self.dateFormat.string(from: Date(timeIntervalSince1970: TimeInterval($0))) } ?? ""
        case "location": text = (item.folder as NSString).abbreviatingWithTildeInPath
        case "note": text = item.note
        default: text = ""
        }
        cell.textField?.stringValue = text
        if case .failed = item.status, id == "status" {
            cell.textField?.textColor = .systemRed
        } else {
            cell.textField?.textColor = .labelColor
        }
        cell.textField?.toolTip = id == "status" || id == "location" || id == "note" ? text : nil
        return cell
    }

    private static func textCell(_ id: NSUserInterfaceItemIdentifier) -> NSTableCellView {
        let cell = NSTableCellView()
        cell.identifier = id
        let field = NSTextField(labelWithString: "")
        field.lineBreakMode = .byTruncatingTail
        field.font = .monospacedDigitSystemFont(ofSize: NSFont.systemFontSize, weight: .regular)
        field.translatesAutoresizingMaskIntoConstraints = false
        cell.addSubview(field)
        cell.textField = field
        NSLayoutConstraint.activate([
            field.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 2),
            field.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -2),
            field.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
        ])
        return cell
    }

    // MARK: actions

    /// A finished download opens its file; an unfinished one opens its Download window (§6).
    @objc func openRow(_ sender: Any?) {
        let row = table.clickedRow
        guard row >= 0, row < shown.count else { return }
        let item = shown[row]
        if item.status.isFinished, let path = item.path, FileManager.default.fileExists(atPath: path) {
            NSWorkspace.shared.open(URL(fileURLWithPath: path))
        } else {
            (NSApp.delegate as? AppDelegate)?.mainWindow?.openDownloadWindow(item.id)
        }
    }

    // MARK: menus

    func menuNeedsUpdate(_ menu: NSMenu) {
        if menu === contextMenu {
            fillContextMenu(menu)
        } else {
            fillColumnsMenu(menu)
        }
    }

    /// The header's context menu and View ▸ Columns: every column but Name, ticked when shown.
    func fillColumnsMenu(_ menu: NSMenu) {
        menu.removeAllItems()
        for column in table.tableColumns where column.identifier.rawValue != "name" {
            let item = menu.addItem(withTitle: column.title, action: #selector(toggleColumn(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = column
            item.state = column.isHidden ? .off : .on
        }
    }

    @objc func toggleColumn(_ sender: NSMenuItem) {
        guard let column = sender.representedObject as? NSTableColumn else { return }
        column.isHidden.toggle()
    }

    /// The download context menu (§4.1). Items that don't apply are hidden (Connections for finished downloads)
    /// or disabled by the window's validation.
    private func fillContextMenu(_ menu: NSMenu) {
        menu.removeAllItems()
        let clicked = table.clickedRow
        guard clicked >= 0, clicked < shown.count else { return }
        if !table.selectedRowIndexes.contains(clicked) {
            table.selectRowIndexes([clicked], byExtendingSelection: false)
        }
        let rows = selectedRows
        typealias W = MainWindowController
        menu.addItem(withTitle: "Open", action: #selector(W.openSelected(_:)), keyEquivalent: "")
        let openWith = menu.addItem(withTitle: "Open With", action: nil, keyEquivalent: "")
        openWith.submenu = openWithMenu(rows)
        menu.addItem(withTitle: "Show in Finder", action: #selector(W.showInFinder(_:)), keyEquivalent: "")
        if rows.count == 1, rows[0].status.isFinished {
            menu.addItem(withTitle: "Rename…", action: #selector(W.renameFile(_:)), keyEquivalent: "")
            menu.addItem(withTitle: "Move To…", action: #selector(W.moveFile(_:)), keyEquivalent: "")
        }
        if rows.count == 1 {
            menu.addItem(withTitle: "Note…", action: #selector(W.editNote(_:)), keyEquivalent: "")
        }
        menu.addItem(.separator())
        menu.addItem(withTitle: "Resume", action: #selector(W.resumeSelected(_:)), keyEquivalent: "")
        menu.addItem(withTitle: "Pause", action: #selector(W.pauseSelected(_:)), keyEquivalent: "")
        menu.addItem(withTitle: "Restart…", action: #selector(W.restartSelected(_:)), keyEquivalent: "")
        menu.addItem(.separator())
        menu.addItem(withTitle: "Add Mirror…", action: #selector(W.addMirror(_:)), keyEquivalent: "")
        if !rows.contains(where: { $0.status.isFinished }) {
            let connections = menu.addItem(withTitle: "Connections", action: nil, keyEquivalent: "")
            let sub = NSMenu(title: "Connections")
            sub.addItem(withTitle: "Add Connection", action: #selector(W.addConnection(_:)), keyEquivalent: "")
            sub.addItem(withTitle: "Remove Connection", action: #selector(W.removeConnection(_:)), keyEquivalent: "")
            if let row = rows.first, rows.count == 1 {
                sub.addItem(.separator())
                let allowed = sub.addItem(withTitle: "\(row.activeConnections) of \(row.allowedConnections) connections in use",
                                          action: nil, keyEquivalent: "")
                allowed.isEnabled = false
                for line in row.live {
                    sub.addItem(withTitle: line, action: nil, keyEquivalent: "").isEnabled = false
                }
            }
            connections.submenu = sub
        }
        menu.addItem(.separator())
        menu.addItem(withTitle: "Copy Link", action: #selector(W.copyLink(_:)), keyEquivalent: "")
        menu.addItem(withTitle: "Properties", action: #selector(W.showProperties(_:)), keyEquivalent: "")
        menu.addItem(.separator())
        menu.addItem(withTitle: "Delete…", action: #selector(W.deleteSelected(_:)), keyEquivalent: "")
    }

    /// The apps that can open the file, the default one first.
    private func openWithMenu(_ rows: [DownloadRow]) -> NSMenu {
        let menu = NSMenu(title: "Open With")
        guard rows.count == 1, let path = rows[0].path, FileManager.default.fileExists(atPath: path) else {
            menu.addItem(withTitle: "No File Yet", action: nil, keyEquivalent: "").isEnabled = false
            return menu
        }
        let url = URL(fileURLWithPath: path)
        let preferred = NSWorkspace.shared.urlForApplication(toOpen: url)
        var apps = NSWorkspace.shared.urlsForApplications(toOpen: url)
        if let preferred { apps = [preferred] + apps.filter { $0 != preferred } }
        for app in apps.prefix(20) {
            let name = FileManager.default.displayName(atPath: app.path)
            let item = menu.addItem(withTitle: app == preferred ? "\(name) (default)" : name,
                                    action: #selector(MainWindowController.openWith(_:)), keyEquivalent: "")
            item.representedObject = app
            let icon = NSWorkspace.shared.icon(forFile: app.path)
            icon.size = NSSize(width: 16, height: 16)
            item.image = icon
        }
        if apps.isEmpty {
            menu.addItem(withTitle: "No Apps", action: nil, keyEquivalent: "").isEnabled = false
        }
        return menu
    }
}

/// L1: the file's icon, its name, and a thin progress bar under the name while unfinished.
final class NameCell: NSTableCellView {
    static let id = NSUserInterfaceItemIdentifier("NameCell")
    private let bar = NSProgressIndicator()
    /// The name sits a little high when the progress bar shows under it, and in the middle of the row otherwise,
    /// level with the other columns.
    private var nameAboveBar: NSLayoutConstraint!
    private var nameCentered: NSLayoutConstraint!

    init() {
        super.init(frame: .zero)
        identifier = Self.id
        let image = NSImageView()
        let title = NSTextField(labelWithString: "")
        title.lineBreakMode = .byTruncatingMiddle
        bar.style = .bar
        bar.controlSize = .mini
        bar.minValue = 0
        bar.maxValue = 1
        for v in [image, title, bar] {
            v.translatesAutoresizingMaskIntoConstraints = false
            addSubview(v)
        }
        imageView = image
        textField = title
        NSLayoutConstraint.activate([
            image.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 2),
            image.centerYAnchor.constraint(equalTo: centerYAnchor),
            image.widthAnchor.constraint(equalToConstant: 20),
            image.heightAnchor.constraint(equalToConstant: 20),
            title.leadingAnchor.constraint(equalTo: image.trailingAnchor, constant: 6),
            title.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -2),
            bar.leadingAnchor.constraint(equalTo: title.leadingAnchor),
            bar.trailingAnchor.constraint(equalTo: title.trailingAnchor),
            bar.topAnchor.constraint(equalTo: title.bottomAnchor, constant: 1),
            bar.heightAnchor.constraint(equalToConstant: 6),
        ])
        nameAboveBar = title.topAnchor.constraint(equalTo: topAnchor, constant: 3)
        nameCentered = title.centerYAnchor.constraint(equalTo: centerYAnchor)
        nameCentered.isActive = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    func show(_ row: DownloadRow) {
        textField?.stringValue = row.name
        textField?.toolTip = row.url
        let ext = (row.name as NSString).pathExtension
        imageView?.image = NSWorkspace.shared.icon(for: .init(filenameExtension: ext) ?? .data)
        // Shown while unfinished: the fraction when the size is known, moving stripes while running without one.
        bar.isHidden = row.status.isFinished || (row.fraction == nil && !row.status.isRunning)
        // Turn one off before the other on, so the two never hold at once.
        let (on, off) = bar.isHidden ? (nameCentered!, nameAboveBar!) : (nameAboveBar!, nameCentered!)
        off.isActive = false
        on.isActive = true
        bar.isIndeterminate = row.fraction == nil
        if let f = row.fraction { bar.doubleValue = f } else if !bar.isHidden { bar.startAnimation(nil) }
        setAccessibilityLabel("\(row.name), \(row.statusText)")
    }
}
