import AppKit
import HDCore

/// Add Links (SPECIFICATION.md §5.3): the links found in the clipboard, a drop or a batch, each with a checkbox,
/// name, size and folder, a filter field, and "Treat as mirrors of one file". Links, text and link files dropped on
/// the window while it is open join the list as more rows (several mirrors of one file at once, for example).
@MainActor
final class AddLinksWindowController: NSWindowController, NSTableViewDataSource, NSTableViewDelegate,
    NSSearchFieldDelegate {
    private final class Item {
        let url: String
        var checked = true
        var name: String
        var size: UInt64?
        var sizeText = "Checking…"
        /// A stream a page scan found: added as one (§15).
        var stream = false
        init(url: String) {
            self.url = url
            name = URL(string: url)?.lastPathComponent.removingPercentEncoding ?? url
            if name.isEmpty || name == "/" { name = url }
            stream = isStreamLink(url: url)
        }
        /// A file a page scan found, already checked.
        init(found: FoundLink) {
            url = found.url
            name = found.name
            size = found.size
            stream = found.stream
            sizeText = found.size.map { "\(bytesText(bytes: $0)), \(found.kind)" } ?? found.kind
        }
    }

    private var items: [Item]
    private var shown: [Item] = []
    private let table = NSTableView()
    private let filter = NSSearchField()
    private let mirrorsBox = NSButton(checkboxWithTitle: "Treat as mirrors of one file", target: nil, action: nil)
    private let soundBox = NSButton(checkboxWithTitle: "Only the sound of streamed videos", target: nil, action: nil)
    private let summary = NSTextField(labelWithString: "")
    private let folderLabel = NSPathControl()
    private var folder = AppModel.downloadsFolder
    private var probes: [Task<Void, Never>] = []
    private var onClose: (() -> Void)?

    init(links: [String], onClose: @escaping () -> Void) {
        items = links.map(Item.init)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 760, height: 460),
                              styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        window.title = Self.title(links.count)
        window.minSize = NSSize(width: 560, height: 320)
        window.setAccessibilityIdentifier("add-links")
        super.init(window: window)
        self.onClose = onClose
        build()
        if let sort = Self.testSort { table.sortDescriptors = [sort] }
        applyFilter()
        probe(items)
        window.center()
    }

    private static func title(_ n: Int) -> String { n == 1 ? "Add 1 Link" : "Add \(n) Links" }

    /// Links dropped on the window: each one not listed yet becomes a checked row and is checked like the others.
    /// Returns how many were new.
    @discardableResult
    func addLinks(_ links: [String]) -> Int {
        var known = Set(items.map { $0.url.trimmingCharacters(in: .whitespaces) })
        var added: [Item] = []
        for link in links {
            let url = link.trimmingCharacters(in: .whitespaces)
            guard !url.isEmpty, known.insert(url).inserted else { continue }
            added.append(Item(url: url))
        }
        guard !added.isEmpty else { NSSound.beep(); return 0 }
        items += added
        window?.title = Self.title(items.count)
        applyFilter()
        probe(added)
        return added.count
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
            // Click a header to sort by it, again for the reverse order.
            if id != "check" { column.sortDescriptorPrototype = NSSortDescriptor(key: id, ascending: true) }
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
        all.toolTip = "Tick every link shown"
        let none = NSButton(title: "Uncheck All", target: self, action: #selector(checkNone(_:)))
        none.toolTip = "Untick every link shown"
        let top = NSStackView(views: [filter, all, none])
        top.spacing = 8

        mirrorsBox.target = self
        mirrorsBox.action = #selector(mirrorsChanged(_:))
        mirrorsBox.toolTip = "Combine the checked links into one download that uses each of them as a source"
        mirrorsBox.setAccessibilityIdentifier("treat-as-mirrors")
        soundBox.target = self
        soundBox.action = #selector(mirrorsChanged(_:))
        soundBox.toolTip = "Save just the sound of each checked stream (.m4a, or .mp3), as the site sends it"
        soundBox.setAccessibilityIdentifier("sound-only")

        folderLabel.url = folder
        folderLabel.pathStyle = .popUp
        folderLabel.toolTip = "The folder the files are saved to; Choose… picks another"
        folderLabel.setAccessibilityLabel("Save to")
        let choose = NSButton(title: "Choose…", target: self, action: #selector(chooseFolder(_:)))
        choose.toolTip = "Choose the folder the files are saved to"
        let folderRow = NSStackView(views: [NSTextField(labelWithString: "Save to:"), folderLabel, choose])
        folderRow.spacing = 8

        summary.textColor = .secondaryLabelColor
        let cancel = NSButton(title: "Cancel", target: self, action: #selector(cancel(_:)))
        cancel.toolTip = "Close without adding anything (Esc)"
        cancel.keyEquivalent = "\u{1b}"
        let add = NSButton(title: "Add", target: self, action: #selector(addToQueue(_:)))
        add.toolTip = "Add: put them in the queue without starting them"
        let download = NSButton(title: "Download", target: self, action: #selector(download(_:)))
        download.keyEquivalent = "\r"
        download.toolTip = "Download: start them now"
        let buttons = NSStackView(views: [summary, NSView(), cancel, add, download])
        buttons.spacing = 8

        let stack = NSStackView(views: [top, scroll, mirrorsBox, soundBox, folderRow, buttons])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 10
        stack.edgeInsets = NSEdgeInsets(top: 16, left: 16, bottom: 16, right: 16)
        stack.translatesAutoresizingMaskIntoConstraints = false
        let content = DropView()
        content.message.stringValue = "Drop links to add them"
        content.onLinks = { [weak self] links in self?.addLinks(links) }
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
    private func probe(_ queue: [Item]) {
        let manager = AppModel.shared.manager
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
                    case .success(let info) where info.page:
                        // A web page: what's on it takes its place (§15).
                        item.sizeText = "Scanning page…"
                        self.table.reloadData()
                        let scanned = await Task.detached { Result { try manager.scanPage(url: url) } }.value
                        guard !Task.isCancelled else { return }
                        self.replace(page: item, with: scanned)
                    case .success(let info):
                        item.name = info.name
                        item.size = info.size
                        item.sizeText = info.size.map { bytesText(bytes: $0) } ?? "Unknown"
                    case .failure(let error):
                        item.sizeText = errorText(error)
                    }
                    // A name or size found changes the order when the list is sorted.
                    if self.table.sortDescriptors.isEmpty { self.table.reloadData(); self.update() } else { self.applyFilter() }
                }
            })
        }
    }

    /// A web page's row gives way to the files and media found on it, each checked; a page with nothing on it, or
    /// that couldn't be read, stays as an unchecked row that says so.
    private func replace(page item: Item, with scanned: Result<PageScan, Error>) {
        let known = Set(items.map(\.url))
        switch scanned {
        case .success(let scan) where !scan.found.isEmpty:
            let found = scan.found.filter { !known.contains($0.url) }.map(Item.init(found:))
            if let at = items.firstIndex(where: { $0 === item }) { items.replaceSubrange(at...at, with: found) }
            window?.title = Self.title(items.count)
        case .success:
            item.checked = false
            item.sizeText = "Web page: no files found"
        case .failure(let error):
            item.checked = false
            item.sizeText = "Web page: \(errorText(error))"
        }
        applyFilter()
        update()
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

    func tableView(_ tableView: NSTableView, sortDescriptorsDidChange oldDescriptors: [NSSortDescriptor]) {
        applyFilter()
    }

    /// The rows shown: those the filter matches, in the order of the column clicked last (sizes by bytes), which is
    /// also the order they are added in.
    private func applyFilter() {
        if let sort = table.sortDescriptors.first, let column = Self.columns[sort.key ?? ""] {
            let folderText = (folder.path as NSString).abbreviatingWithTildeInPath
            let rows = items.map { LinkSortRow(name: $0.name, size: $0.size, folder: folderText, url: $0.url) }
            items = linkSortOrder(rows: rows, column: column, ascending: sort.ascending).map { items[Int($0)] }
        }
        let needle = filter.stringValue.lowercased()
        shown = needle.isEmpty ? items : items.filter {
            $0.name.lowercased().contains(needle) || $0.url.lowercased().contains(needle)
        }
        table.reloadData()
        update()
    }

    /// The sort a new window starts with, as a click on that column's header gives (the `--sort-links` test hook).
    static var testSort: NSSortDescriptor?

    private static let columns: [String: LinkColumn] = ["name": .name, "size": .size, "folder": .folder, "link": .link]

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
        // Only for streams; a whole video file isn't split.
        soundBox.isEnabled = checked.contains { $0.stream }
        if !soundBox.isEnabled { soundBox.state = .off }
        let n = checked.count
        // Links to the same file (same name, and same size when known) become one download with mirrors.
        let files = Set(checked.map { "\($0.name)\u{0}\($0.size.map(String.init) ?? "")" }).count
        summary.stringValue = mirrorsBox.state == .on
            ? "1 download from \(n) sources"
            : n == 1 ? "1 link checked"
            : files < n ? "\(n) links checked: \(files) download\(files == 1 ? "" : "s") with mirrors"
            : "\(n) links checked"
        summary.toolTip = "Links to the same file (same name and size) are added as one download and its mirrors"

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

    /// Adds the checked links one at a time: links to the same file become one download and its mirrors, and a
    /// different file with a name in use asks Skip, Replace or Keep Both (§5.6).
    private func submit(start: Bool) {
        let chosen = checked
        guard !chosen.isEmpty else { NSSound.beep(); return }
        let connections = AppModel.shared.settings.connections
        let links: [PendingLink] = mirrorsBox.state == .on
            ? [PendingLink(url: chosen[0].url, mirrors: chosen.dropFirst().map(\.url), name: chosen[0].name,
                           size: chosen.compactMap(\.size).first)]
            : chosen.map { PendingLink(url: $0.url, name: $0.name, size: $0.size, stream: $0.stream,
                                       soundOnly: $0.stream && soundBox.state == .on) }
        let folder = self.folder
        close()
        let window = (NSApp.delegate as? AppDelegate)?.mainWindow?.window
        Task { await AppModel.shared.addAsking(links, folder: folder, connections: connections, start: start,
                                               window: window) }
    }

    override func close() {
        probes.forEach { $0.cancel() }
        super.close()
        onClose?()
        onClose = nil
    }
}
