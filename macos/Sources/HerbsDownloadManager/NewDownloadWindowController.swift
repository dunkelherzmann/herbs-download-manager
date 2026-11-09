import AppKit
import HDCore

/// New Download (SPECIFICATION.md §5.1): Link, Mirrors, Name, Save to, Connections and Size, with Cancel, Add
/// and Download (the default).
@MainActor
final class NewDownloadWindowController: NSWindowController, NSTextFieldDelegate {
    private let link = NSTextField()
    private let mirrors = NSTextView()
    private let name = NSTextField()
    private let folderLabel = NSPathControl()
    private let connections = NSPopUpButton()
    private let size = NSTextField(labelWithString: "")
    private var folder = AppModel.downloadsFolder
    /// The user typed a name: the server's no longer replaces it.
    private var nameEdited = false
    /// The size the server gave for the link, used to match it with a download already in the list.
    private var probedSize: UInt64?
    private var probeTask: Task<Void, Never>?
    private var onClose: (() -> Void)?

    init(link initial: String?, onClose: @escaping () -> Void) {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 560, height: 360),
                              styleMask: [.titled, .closable], backing: .buffered, defer: false)
        window.title = "New Download"
        window.setAccessibilityIdentifier("new-download")
        super.init(window: window)
        self.onClose = onClose
        build()
        if let initial { link.stringValue = initial; linkChanged() }
        window.center()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    private func build() {
        guard let window else { return }
        link.placeholderString = "https://example.com/file.zip"
        link.delegate = self
        link.setAccessibilityIdentifier("link")
        link.toolTip = "Link: the address of the file to download"

        mirrors.isRichText = false
        mirrors.font = .systemFont(ofSize: NSFont.systemFontSize)
        mirrors.isAutomaticQuoteSubstitutionEnabled = false
        mirrors.isAutomaticLinkDetectionEnabled = false
        mirrors.setAccessibilityIdentifier("mirrors")
        mirrors.setAccessibilityLabel("Mirrors")
        mirrors.toolTip = "Mirrors: other servers with the same file, one address per line (optional)"
        let mirrorScroll = NSScrollView()
        mirrorScroll.documentView = mirrors
        mirrorScroll.hasVerticalScroller = true
        mirrorScroll.borderType = .bezelBorder
        mirrors.autoresizingMask = [.width]
        mirrors.isVerticallyResizable = true
        mirrors.textContainer?.widthTracksTextView = true
        mirrorScroll.heightAnchor.constraint(equalToConstant: 64).isActive = true

        name.placeholderString = "From the server"
        name.delegate = self
        name.setAccessibilityIdentifier("name")
        name.toolTip = "Name: the file name to save as"

        folderLabel.url = folder
        folderLabel.pathStyle = .popUp
        folderLabel.isEditable = false
        let choose = NSButton(title: "Choose…", target: self, action: #selector(chooseFolder(_:)))
        choose.toolTip = "Choose the folder to save to"
        let folderRow = NSStackView(views: [folderLabel, choose])
        folderRow.spacing = 8

        for n in connectionChoices() { connections.addItem(withTitle: "\(n)") }
        connections.selectItem(withTitle: "\(defaultSettings().connections)")
        connections.setAccessibilityIdentifier("connections")
        connections.toolTip = "Connections: how many parts of the file download at once (1–32)"

        size.textColor = .secondaryLabelColor
        size.setAccessibilityIdentifier("size")

        let grid = NSGridView(views: [
            [label("Link:"), link],
            [label("Mirrors:"), mirrorScroll],
            [label("Name:"), name],
            [label("Save to:"), folderRow],
            [label("Connections:"), connections],
            [label("Size:"), size],
        ])
        grid.rowSpacing = 10
        grid.columnSpacing = 8
        grid.column(at: 0).xPlacement = .trailing
        grid.rowAlignment = .firstBaseline
        grid.row(at: 1).yPlacement = .top
        grid.cell(for: mirrorScroll)?.yPlacement = .top

        let cancel = NSButton(title: "Cancel", target: self, action: #selector(cancel(_:)))
        cancel.keyEquivalent = "\u{1b}"
        let add = NSButton(title: "Add", target: self, action: #selector(addToQueue(_:)))
        add.toolTip = "Add: put it in the queue without starting it"
        let download = NSButton(title: "Download", target: self, action: #selector(download(_:)))
        download.keyEquivalent = "\r"
        download.toolTip = "Download: start it now"
        let buttons = NSStackView(views: [NSView(), cancel, add, download])
        buttons.spacing = 8

        let stack = NSStackView(views: [grid, buttons])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 18
        stack.edgeInsets = NSEdgeInsets(top: 20, left: 20, bottom: 20, right: 20)
        stack.translatesAutoresizingMaskIntoConstraints = false
        let content = NSView()
        content.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            stack.topAnchor.constraint(equalTo: content.topAnchor),
            stack.bottomAnchor.constraint(equalTo: content.bottomAnchor),
            grid.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -40),
            buttons.widthAnchor.constraint(equalTo: grid.widthAnchor),
            link.widthAnchor.constraint(greaterThanOrEqualToConstant: 400),
        ])
        window.contentView = content
        window.initialFirstResponder = link
        // As tall as the fields need, no taller.
        content.layoutSubtreeIfNeeded()
        window.setContentSize(NSSize(width: window.frame.width, height: content.fittingSize.height))
    }

    private func label(_ text: String) -> NSTextField {
        NSTextField(labelWithString: text)
    }

    // MARK: link checking

    func controlTextDidChange(_ notification: Notification) {
        if notification.object as? NSTextField === name {
            nameEdited = !name.stringValue.isEmpty
        } else {
            linkChanged()
        }
    }

    /// Asks the server about the link half a second after typing stops: its name and size (A3, A6).
    private func linkChanged() {
        probeTask?.cancel()
        probedSize = nil
        let url = link.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !url.isEmpty else { size.stringValue = ""; return }
        size.stringValue = "Checking…"
        let manager = AppModel.shared.manager
        probeTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(500))
            guard !Task.isCancelled else { return }
            let result = await Task.detached { Result { try manager.probe(url: url) } }.value
            guard !Task.isCancelled, let self else { return }
            switch result {
            case .success(let info):
                self.probedSize = info.size
                if !self.nameEdited { self.name.stringValue = info.name }
                let total = info.size.map { bytesText(bytes: $0) }
                self.size.stringValue = switch (total, info.resumable) {
                case (let t?, true): "\(t), resumable"
                case (let t?, false): "\(t), can't resume"
                case (nil, _): "size unknown, can't resume"
                }
            case .failure(let error):
                self.size.stringValue = errorText(error)
            }
        }
    }

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
                }
            }
        }
    }

    @objc func cancel(_ sender: Any?) { close() }
    @objc func addToQueue(_ sender: Any?) { submit(start: false) }
    @objc func download(_ sender: Any?) { submit(start: true) }

    /// Adds the download; if a file of that name exists, offers a numbered name (first), Replace, or Show.
    private func submit(start: Bool) {
        let url = link.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !findLinks(text: url).isEmpty, let window else {
            NSSound.beep()
            size.stringValue = "Enter a web link (http, https or ftp)"
            window?.makeFirstResponder(link)
            return
        }
        let mirrorLinks = findLinks(text: mirrors.string).filter { $0 != url }
        let chosen = name.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        let count = UInt32(connections.titleOfSelectedItem ?? "") ?? defaultSettings().connections
        let finish = { (finalName: String?) in
            AppModel.shared.add(url: url, mirrors: mirrorLinks, folder: self.folder, name: finalName,
                                size: self.probedSize, connections: count, start: start)
            self.close()
        }
        guard !chosen.isEmpty else { return finish(nil) }
        let existing = folder.appendingPathComponent(chosen)
        guard FileManager.default.fileExists(atPath: existing.path) else { return finish(chosen) }
        let numbered = freeName(folder: folder.path, name: chosen)
        let alert = NSAlert()
        alert.messageText = "“\(chosen)” already exists in \(folder.lastPathComponent)."
        alert.informativeText = "Save the download under another name, or replace the existing file?"
        alert.addButton(withTitle: "Save as “\(numbered)”")
        alert.addButton(withTitle: "Replace")
        alert.addButton(withTitle: "Show Existing File")
        alert.addButton(withTitle: "Cancel")
        alert.buttons[1].hasDestructiveAction = true
        alert.beginSheetModal(for: window) { response in
            MainActor.assumeIsolated {
                switch response {
                case .alertFirstButtonReturn: finish(numbered)
                case .alertSecondButtonReturn:
                    // The old file goes to the Trash, so replacing can be undone.
                    try? FileManager.default.trashItem(at: existing, resultingItemURL: nil)
                    finish(chosen)
                case .alertThirdButtonReturn: NSWorkspace.shared.activateFileViewerSelecting([existing])
                default: break
                }
            }
        }
    }

    override func close() {
        probeTask?.cancel()
        super.close()
        onClose?()
        onClose = nil
    }
}
