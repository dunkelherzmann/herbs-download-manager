import AppKit
import HDCore

/// The sidebar (SPECIFICATION.md §2.1): the five status filters with their counts, each with the fixed file-type
/// groups under it.
@MainActor
final class SidebarViewController: NSViewController, NSOutlineViewDataSource, NSOutlineViewDelegate {
    /// One line of the sidebar: a filter, or a file-type group under a filter.
    final class Item: NSObject {
        let filter: Filter
        let kind: FileKind?
        init(filter: Filter, kind: FileKind?) { self.filter = filter; self.kind = kind }
    }

    static let filters: [Filter] = [.all, .downloading, .paused, .finished, .unfinished]
    static let kinds: [FileKind] = [.compressed, .documents, .music, .pictures, .video, .programs, .other]

    var onSelect: ((Filter, FileKind?) -> Void)?
    private let outline = NSOutlineView()
    private lazy var items: [Item] = Self.filters.map { Item(filter: $0, kind: nil) }
    private lazy var groups: [Filter: [Item]] = Dictionary(uniqueKeysWithValues: Self.filters.map { f in
        (f, Self.kinds.map { Item(filter: f, kind: $0) })
    })

    override func loadView() {
        let column = NSTableColumn(identifier: .init("filter"))
        outline.addTableColumn(column)
        outline.outlineTableColumn = column
        outline.headerView = nil
        outline.style = .sourceList
        outline.rowSizeStyle = .default
        outline.dataSource = self
        outline.delegate = self
        outline.setAccessibilityIdentifier("sidebar")
        outline.setAccessibilityLabel("Filters")
        let scroll = NSScrollView()
        scroll.documentView = outline
        scroll.hasVerticalScroller = true
        scroll.drawsBackground = false
        view = scroll
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        outline.reloadData()
        outline.selectRowIndexes([0], byExtendingSelection: false)
        AppModel.shared.observe { [weak self] in self?.updateCounts() }
    }

    private func count(_ item: Item) -> Int {
        AppModel.shared.rows.filter { row in
            filterMatches(filter: item.filter, status: row.status) && (item.kind == nil || row.kind == item.kind)
        }.count
    }

    private func updateCounts() {
        for row in 0..<outline.numberOfRows {
            guard let item = outline.item(atRow: row) as? Item,
                  let cell = outline.view(atColumn: 0, row: row, makeIfNecessary: false) as? FilterCell else { continue }
            cell.count.stringValue = "\(count(item))"
        }
    }

    // MARK: data source

    func outlineView(_ outlineView: NSOutlineView, numberOfChildrenOfItem item: Any?) -> Int {
        guard let item = item as? Item else { return items.count }
        return item.kind == nil ? groups[item.filter]?.count ?? 0 : 0
    }

    func outlineView(_ outlineView: NSOutlineView, child index: Int, ofItem item: Any?) -> Any {
        guard let item = item as? Item else { return items[index] }
        return groups[item.filter]![index]
    }

    func outlineView(_ outlineView: NSOutlineView, isItemExpandable item: Any) -> Bool {
        (item as? Item)?.kind == nil
    }

    // MARK: delegate

    func outlineView(_ outlineView: NSOutlineView, viewFor tableColumn: NSTableColumn?, item: Any) -> NSView? {
        guard let item = item as? Item else { return nil }
        let cell = (outlineView.makeView(withIdentifier: FilterCell.id, owner: nil) as? FilterCell) ?? FilterCell()
        let title = item.kind.map { fileKindTitle(kind: $0) } ?? filterTitle(filter: item.filter)
        cell.textField?.stringValue = title
        cell.imageView?.image = NSImage(systemSymbolName: Self.symbol(item), accessibilityDescription: nil)
        cell.count.stringValue = "\(count(item))"
        cell.setAccessibilityLabel(title)
        cell.setAccessibilityIdentifier("filter-\(title)" + (item.kind == nil ? "" : "-\(filterTitle(filter: item.filter))"))
        return cell
    }

    func outlineViewSelectionDidChange(_ notification: Notification) {
        guard let item = outline.item(atRow: outline.selectedRow) as? Item else { return }
        onSelect?(item.filter, item.kind)
    }

    private static func symbol(_ item: Item) -> String {
        if let kind = item.kind {
            switch kind {
            case .compressed: return "doc.zipper"
            case .documents: return "doc.text"
            case .music: return "music.note"
            case .pictures: return "photo"
            case .video: return "film"
            case .programs: return "app.badge"
            case .other: return "doc"
            }
        }
        switch item.filter {
        case .all: return "folder"
        case .downloading: return "arrow.down"
        case .paused: return "pause"
        case .finished: return "checkmark.circle"
        case .unfinished: return "clock.arrow.circlepath"
        }
    }
}

/// A sidebar line: icon, title and count at the right.
final class FilterCell: NSTableCellView {
    static let id = NSUserInterfaceItemIdentifier("FilterCell")
    let count = NSTextField(labelWithString: "")

    init() {
        super.init(frame: .zero)
        identifier = Self.id
        let image = NSImageView()
        let title = NSTextField(labelWithString: "")
        title.lineBreakMode = .byTruncatingTail
        count.textColor = .secondaryLabelColor
        count.font = .monospacedDigitSystemFont(ofSize: NSFont.smallSystemFontSize, weight: .regular)
        count.alignment = .right
        for v in [image, title, count] {
            v.translatesAutoresizingMaskIntoConstraints = false
            addSubview(v)
        }
        imageView = image
        textField = title
        NSLayoutConstraint.activate([
            image.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 2),
            image.centerYAnchor.constraint(equalTo: centerYAnchor),
            image.widthAnchor.constraint(equalToConstant: 18),
            title.leadingAnchor.constraint(equalTo: image.trailingAnchor, constant: 6),
            title.centerYAnchor.constraint(equalTo: centerYAnchor),
            count.leadingAnchor.constraint(greaterThanOrEqualTo: title.trailingAnchor, constant: 6),
            count.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -6),
            count.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
        title.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }
}
