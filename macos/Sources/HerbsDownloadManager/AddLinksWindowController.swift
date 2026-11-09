import AppKit
import HDCore

/// Add Links (SPECIFICATION.md §5.3): the links found in the clipboard, a drop or a batch, each with a checkbox,
/// name, size and folder, a filter field, and "Treat as mirrors of one file".
@MainActor
final class AddLinksWindowController: NSWindowController, NSTableViewDataSource, NSTableViewDelegate,
    NSSearchFieldDelegate {
    private final class Item {
        let url: String
        var checked = true
        var name: String
        var size: UInt64?
        var sizeText = "Checking…"
        init(url: String) {
            self.url = url
            name = URL(string: url)?.lastPathComponent.removingPercentEncoding ?? url
            if name.isEmpty || name == "/" { name = url }
        }
    }

    private let items: [Item]
    private var shown: [Item] = []
    private let table = NSTableView()
    private let filter = NSSearchField()
    private let mirrorsBox = NSButton(checkboxWithTitle: "Treat as mirrors of one file", target: nil, action: nil)
    private let summary = NSTextField(labelWithString: "")
    private let folderLabel = NSPathControl()
    private var folder = AppModel.downloadsFolder
    private var probes: [Task<Void, Never>] = []
    private var onClose: (() -> Void)?

    init(links: [String], onClose: @escaping () -> Void) {
        items = links.map(Item.init)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 760, height: 460),
                              styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        window.title = links.count == 1 ? "Add 1 Link" : "Add \(links.count) Links"
        window.minSize = NSSize(width: 560, height: 320)
        window.setAccessibilityIdentifier("add-links")
        super.init(window: window)
        self.onClose = onClose
        build()
        applyFilter()
        probeAll()
        window.center()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    private func build() {
        guard let window else { return }
        for (id, title, width) in [("check", "", 24.0), ("name", "Name", 260), ("size", "Size", 110),
                                   ("folder", "Folder", 140), ("link", "Link", 260)] {
            let column = NSTableColumn(identifier: .init(id))
            column.title = title
            column.width = width
            table.addTableColumn(column)
        }
        table.usesAlternatingRowBackgroundColors = true
        table.allowsMultipleSelection = true
        table.dataSource = self
        table.delegate = self
        table.setAccessibilityIdentifier("links")
        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.borderType = .bezelBorder

        filter.placeholderString = "Filter"
        filter.delegate = self
        filter.toolTip = "Filter: show links whose name or address contains the text"
        let all = NSButton(title: "Check All", target: self, action: #selector(checkAll(_:)))
        let none = NSButton(title: "Uncheck All", target: self, action: #selector(checkNone(_:)))
        let top = NSStackView(views: [filter, all, none])
        top.spacing = 8

        mirrorsBox.target = self
        mirrorsBox.action = #selector(mirrorsChanged(_:))
        mirrorsBox.toolTip = "Combine the checked links into one download that uses each of them as a source"
        mirrorsBox.setAccessibilityIdentifier("treat-as-mirrors")

        folderLabel.url = folder
        folderLabel.pathStyle = .popUp
        let choose = NSButton(title: "Choose…", target: self, action: #selector(chooseFolder(_:)))
        let folderRow = NSStackView(views: [NSTextField(labelWithString: "Save to:"), folderLabel, choose])
        folderRow.spacing = 8

        summary.textColor = .secondaryLabelColor
        let cancel = NSButton(title: "Cancel", target: self, action: #selector(cancel(_:)))
        cancel.keyEquivalent = "\u{1b}"
        let add = NSButton(title: "Add", target: self, action: #selector(addToQueue(_:)))
        add.toolTip = "Add: put them in the queue without starting them"
        let download = NSButton(title: "Download", target: self, action: #selector(download(_:)))
        download.keyEquivalent = "\r"
        download.toolTip = "Download: start them now"
        let buttons = NSStackView(views: [summary, NSView(), cancel, add, download])
        buttons.spacing = 8

        let stack = NSStackView(views: [top, scroll, mirrorsBox, folderRow, buttons])
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
            top.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -32),
            scroll.widthAnchor.constraint(equalTo: top.widthAnchor),
            buttons.widthAnchor.constraint(equalTo: top.widthAnchor),
            scroll.heightAnchor.constraint(greaterThanOrEqualToConstant: 200),
        ])
        window.contentView = content
    }

    // MARK: names and sizes

    /// Asks each server for the file's name and size, four at a time.
    private func probeAll() {
        let manager = AppModel.shared.manager
        let queue = items
        var next = 0
        for _ in 0..<min(4, queue.count) {
            probes.append(Task { [weak self] in
                while !Task.isCancelled {
                    guard next < queue.count else { return }
                    let item = queue[next]
                    next += 1
                    let url = item.url
                    let result = await Task.detached { Result { try manager.probe(url: url) } }.value
                    guard let self, !Task.isCancelled else { return }
                    switch result {
                    case .success(let info):
                        item.name = info.name
                        item.size = info.size
                        item.sizeText = info.size.map { bytesText(bytes: $0) } ?? "Unknown"
                    case .failure(let error):
                        item.sizeText = errorText(error)
                    }
                    self.table.reloadData()
                    self.update()
                }
            })
        }
    }

    // MARK: table

    func numberOfRows(in tableView: NSTableView) -> Int { shown.count }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard let id = tableColumn?.identifier.rawValue, row < shown.count else { return nil }
        let item = shown[row]
        if id == "check" {
            let box = NSButton(checkboxWithTitle: "", target: self, action: #selector(toggle(_:)))
            box.state = item.checked ? .on : .off
            box.tag = row
            box.setAccessibilityLabel("Download \(item.name)")
            return box
        }
        let text: String = switch id {
        case "name": item.name
        case "size": item.sizeText
        case "folder": (folder.path as NSString).abbreviatingWithTildeInPath
        default: item.url
        }
        let field = NSTextField(labelWithString: text)
        field.lineBreakMode = id == "link" ? .byTruncatingMiddle : .byTruncatingTail
        field.toolTip = text
        return field
    }

    @objc func toggle(_ sender: NSButton) {
        guard sender.tag < shown.count else { return }
        shown[sender.tag].checked = sender.state == .on
        update()
    }

    @objc func checkAll(_ sender: Any?) { shown.forEach { $0.checked = true }; table.reloadData(); update() }
    @objc func checkNone(_ sender: Any?) { shown.forEach { $0.checked = false }; table.reloadData(); update() }

    func controlTextDidChange(_ notification: Notification) { applyFilter() }

    private func applyFilter() {
        let needle = filter.stringValue.lowercased()
        shown = needle.isEmpty ? items : items.filter {
            $0.name.lowercased().contains(needle) || $0.url.lowercased().contains(needle)
        }
        table.reloadData()
        update()
    }

    private var checked: [Item] { items.filter(\.checked) }

    /// "Treat as mirrors" is offered when the checked links share a file name or a size.
    private var canBeMirrors: Bool {
        let c = checked
        guard c.count > 1 else { return false }
        let sameName = Set(c.map(\.name)).count == 1
        let sizes = c.compactMap(\.size)
        let sameSize = sizes.count == c.count && Set(sizes).count == 1
        return sameName || sameSize
    }

    private func update() {
        mirrorsBox.isEnabled = canBeMirrors
        if !canBeMirrors { mirrorsBox.state = .off }
        let n = checked.count
        summary.stringValue = mirrorsBox.state == .on
            ? "1 download from \(n) sources"
            : (n == 1 ? "1 link checked" : "\(n) links checked")
    }

    @objc func mirrorsChanged(_ sender: Any?) { update() }

    // MARK: actions

    @objc func chooseFolder(_ sender: Any?) {
        guard let window else { return }
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = true
        panel.directoryURL = folder
        panel.prompt = "Choose"
        panel.beginSheetModal(for: window) { response in
            MainActor.assumeIsolated {
                if response == .OK, let url = panel.url {
                    self.folder = url
                    self.folderLabel.url = url
                    self.table.reloadData()
                }
            }
        }
    }

    @objc func cancel(_ sender: Any?) { close() }
    @objc func addToQueue(_ sender: Any?) { submit(start: false) }
    @objc func download(_ sender: Any?) { submit(start: true) }

    private func submit(start: Bool) {
        let chosen = checked
        guard !chosen.isEmpty else { NSSound.beep(); return }
        let connections = defaultSettings().connections
        if mirrorsBox.state == .on {
            AppModel.shared.add(url: chosen[0].url, mirrors: chosen.dropFirst().map(\.url), folder: folder,
                                name: nil, connections: connections, start: start)
        } else {
            for item in chosen {
                AppModel.shared.add(url: item.url, mirrors: [], folder: folder, name: nil,
                                    connections: connections, start: start)
            }
        }
        close()
    }

    override func close() {
        probes.forEach { $0.cancel() }
        super.close()
        onClose?()
        onClose = nil
    }
}
