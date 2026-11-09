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
    private(set) var shown: [DownloadRow] = []
    private let empty = NSTextField(labelWithString: "No downloads yet")

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
        table.autosaveName = "DownloadList"
        table.autosaveTableColumns = true
        table.setAccessibilityIdentifier("downloads")
        table.setAccessibilityLabel("Downloads")
        let headerMenu = NSMenu()
        headerMenu.delegate = self
        table.headerView?.menu = headerMenu

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
        let needle = search.trimmingCharacters(in: .whitespaces).lowercased()
        var rows = AppModel.shared.rows.filter { row in
            filterMatches(filter: filter, status: row.status)
                && (kind == nil || row.kind == kind)
                && (needle.isEmpty || row.name.lowercased().contains(needle) || row.url.lowercased().contains(needle))
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
        default: return .orderedSame
        }
    }

    func numberOfRows(in tableView: NSTableView) -> Int { shown.count }

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
        default: text = ""
        }
        cell.textField?.stringValue = text
        if case .failed = item.status, id == "status" {
            cell.textField?.textColor = .systemRed
        } else {
            cell.textField?.textColor = .labelColor
        }
        cell.textField?.toolTip = id == "status" || id == "location" ? text : nil
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

    /// A finished download opens its file; an unfinished one will open its Download window (§6).
    @objc func openRow(_ sender: Any?) {
        let row = table.clickedRow
        guard row >= 0, row < shown.count else { return }
        let item = shown[row]
        if item.status.isFinished, let path = item.path {
            NSWorkspace.shared.open(URL(fileURLWithPath: path))
        }
    }

    // MARK: header menu: show and hide columns

    func menuNeedsUpdate(_ menu: NSMenu) {
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
}

/// L1: the file's icon, its name, and a thin progress bar under the name while unfinished.
final class NameCell: NSTableCellView {
    static let id = NSUserInterfaceItemIdentifier("NameCell")
    private let bar = NSProgressIndicator()

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
            title.topAnchor.constraint(equalTo: topAnchor, constant: 3),
            bar.leadingAnchor.constraint(equalTo: title.leadingAnchor),
            bar.trailingAnchor.constraint(equalTo: title.trailingAnchor),
            bar.topAnchor.constraint(equalTo: title.bottomAnchor, constant: 1),
            bar.heightAnchor.constraint(equalToConstant: 6),
        ])
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
        bar.isIndeterminate = row.fraction == nil
        if let f = row.fraction { bar.doubleValue = f } else if !bar.isHidden { bar.startAnimation(nil) }
        setAccessibilityLabel("\(row.name), \(row.statusText)")
    }
}
