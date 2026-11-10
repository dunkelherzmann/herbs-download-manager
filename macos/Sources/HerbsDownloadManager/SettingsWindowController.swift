import AppKit
import HDCore

/// A speed limit: "Limit to [ 500 ] [KB/s ▾]". Off when the box is unticked or the number is empty or 0. Decimal
/// units, as the app shows speeds (1 KB/s = 1,000 bytes a second).
final class SpeedLimitControl: NSStackView, NSTextFieldDelegate {
    let box = NSButton(checkboxWithTitle: "Limit to", target: nil, action: nil)
    let number = NSTextField()
    let unit = NSPopUpButton()
    /// Called with the new limit in bytes a second (`nil`: none) whenever it changes.
    var onChange: ((UInt64?) -> Void)?

    init(what: String) {
        super.init(frame: .zero)
        orientation = .horizontal
        spacing = 6
        box.target = self
        box.action = #selector(changed(_:))
        box.toolTip = "Limit the download speed of \(what)"
        number.formatter = {
            let f = NumberFormatter()
            f.numberStyle = .decimal
            f.minimum = 1
            f.maximum = 1_000_000
            f.maximumFractionDigits = 1
            return f
        }()
        number.delegate = self
        number.target = self
        number.action = #selector(changed(_:))
        number.placeholderString = "500"
        number.widthAnchor.constraint(equalToConstant: 72).isActive = true
        number.toolTip = "The most \(what) may download each second"
        number.setAccessibilityLabel("Speed limit")
        unit.addItems(withTitles: ["KB/s", "MB/s"])
        unit.target = self
        unit.action = #selector(changed(_:))
        unit.toolTip = "Kilobytes or megabytes a second"
        unit.setAccessibilityLabel("Speed limit unit")
        [box, number, unit].forEach(addArrangedSubview)
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    /// The limit in bytes a second, or `nil` for none.
    var limit: UInt64? {
        get {
            guard box.state == .on, number.doubleValue > 0 else { return nil }
            return UInt64(number.doubleValue * (unit.indexOfSelectedItem == 1 ? 1_000_000 : 1_000))
        }
        set {
            box.state = newValue == nil ? .off : .on
            if let v = newValue {
                let mb = v >= 1_000_000 && v % 100_000 == 0
                unit.selectItem(at: mb ? 1 : 0)
                number.doubleValue = Double(v) / (mb ? 1_000_000 : 1_000)
            } else if number.stringValue.isEmpty {
                number.stringValue = ""
            }
            enableNumber()
        }
    }

    var isEnabled: Bool = true {
        didSet { box.isEnabled = isEnabled; enableNumber() }
    }

    private func enableNumber() {
        number.isEnabled = isEnabled && box.state == .on
        unit.isEnabled = number.isEnabled
    }

    @objc private func changed(_ sender: Any?) {
        if sender as? NSButton === box, box.state == .on, number.doubleValue <= 0 {
            number.doubleValue = 500
            window?.makeFirstResponder(number)
        }
        enableNumber()
        onChange?(limit)
    }

    func controlTextDidEndEditing(_ obj: Notification) { changed(number) }
}

/// Settings (§9, ⌘,): one pane, applied as soon as something changes, as macOS settings are.
@MainActor
final class SettingsWindowController: NSWindowController {
    static let shared = SettingsWindowController()

    private let folderLabel = NSTextField(labelWithString: "")
    private let connections = NSPopUpButton()
    private let perServer = NSPopUpButton()
    private let atOnce = NSPopUpButton()
    private let speed = SpeedLimitControl(what: "all downloads together")
    private let retries = NSPopUpButton()
    private let whenExists = NSPopUpButton()
    private let signInsText = NSTextField(labelWithString: "")
    private let manageSignIns = NSButton(title: "Manage…", target: nil, action: nil)
    private let hostLimits = NSTextField(labelWithString: "")
    private let forgetLimits = NSButton(title: "Forget", target: nil, action: nil)
    private let serverDate = NSButton(checkboxWithTitle: "Keep the server's date for finished files", target: nil, action: nil)
    private let checksumFiles = NSButton(checkboxWithTitle: "Look for checksum files next to the link", target: nil, action: nil)
    private let notify = NSButton(checkboxWithTitle: "Show a notification when a download finishes", target: nil,
                                  action: nil)

    private static let existsChoices: [(String, WhenFileExists)] = [
        ("Ask", .ask), ("Keep both", .keepBoth), ("Skip the new link", .skip), ("Replace the existing file", .replace),
    ]

    private init() {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 520, height: 300),
                              styleMask: [.titled, .closable], backing: .buffered, defer: true)
        window.title = "Settings"
        window.setFrameAutosaveName(TestCopy.autosave("Settings"))
        super.init(window: window)
        build()
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    func show() {
        load()
        if window?.isVisible != true { window?.center() }
        showWindow(nil)
        window?.makeKeyAndOrderFront(nil)
    }

    private func popUp(_ p: NSPopUpButton, _ values: ClosedRange<Int>, id: String, name: String = "", help: String) {
        p.setAccessibilityLabel(name)
        p.removeAllItems()
        for n in values { p.addItem(withTitle: "\(n)") }
        p.target = self
        p.action = #selector(save(_:))
        p.toolTip = help
        p.setAccessibilityIdentifier(id)
    }

    private func build() {
        folderLabel.lineBreakMode = .byTruncatingMiddle
        folderLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        folderLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 260).isActive = true
        folderLabel.setAccessibilityIdentifier("settings-folder")
        let choose = NSButton(title: "Choose…", target: self, action: #selector(chooseFolder(_:)))
        choose.toolTip = "Choose the folder new downloads are saved to"
        let reset = NSButton(title: "Use Downloads", target: self, action: #selector(useDownloads(_:)))
        reset.toolTip = "Save new downloads to your Downloads folder"
        reset.setAccessibilityIdentifier("settings-use-downloads")
        let folderRow = NSStackView(views: [folderLabel, choose, reset])
        folderRow.spacing = 6

        popUp(connections, 1...32, id: "settings-connections", name: "Connections per download",
              help: "Connections a new download uses at most (1–32); New Download can choose another number")
        popUp(perServer, 1...32, id: "settings-per-server", name: "Connections per server",
              help: "Connections a download opens to any one server (1–32), from its next start")
        popUp(atOnce, 1...10, id: "settings-at-once", name: "Downloads at once", help: "Downloads the queue runs at the same time (1–10)")
        popUp(retries, 0...20, id: "settings-retries", name: "Retries per source",
              help: "Times a link or mirror is tried again after failing before it counts as failed (0–20)")
        whenExists.addItems(withTitles: Self.existsChoices.map(\.0))
        whenExists.target = self
        whenExists.action = #selector(save(_:))
        whenExists.toolTip = "What happens when a different file already has a new download's name"
        whenExists.setAccessibilityIdentifier("settings-when-exists")
        whenExists.setAccessibilityLabel("When a file exists")
        speed.onChange = { [weak self] _ in self?.save(nil) }
        speed.box.setAccessibilityIdentifier("settings-speed-limit")
        manageSignIns.target = self
        manageSignIns.action = #selector(openSignIns(_:))
        manageSignIns.toolTip = "See, add and change the user names, passwords, cookies and referers saved for sites, and the Administrator Password that encrypts them"
        manageSignIns.setAccessibilityIdentifier("settings-sign-ins")
        forgetLimits.target = self
        forgetLimits.action = #selector(forgetHostLimits(_:))
        forgetLimits.toolTip = "Forget every server's remembered limit; downloads learn them again"
        forgetLimits.setAccessibilityIdentifier("settings-forget-limits")
        checksumFiles.target = self
        checksumFiles.action = #selector(save(_:))
        checksumFiles.toolTip = "When a download has no checksum set, look for <file>.sha256 (or .sha512, .sha1, .md5) "
            + "next to its link and check the file with it"
        checksumFiles.setAccessibilityIdentifier("settings-checksum-files")
        serverDate.target = self
        serverDate.action = #selector(save(_:))
        serverDate.toolTip = "A finished file gets the date the server gives it (Last-Modified) instead of the time it finished downloading"
        serverDate.setAccessibilityIdentifier("settings-server-date")
        notify.target = self
        notify.action = #selector(save(_:))
        notify.toolTip = "Show a macOS notification each time a download finishes"
        notify.setAccessibilityIdentifier("settings-notify")

        let label = { (text: String) in NSTextField(labelWithString: text) }
        let grid = NSGridView(views: [
            [label("Save downloads to:"), folderRow],
            [label("Connections per download:"), connections],
            [label("Connections per server:"), perServer],
            [label("Downloads at once:"), atOnce],
            [label("Speed limit:"), speed],
            [label("Retries per source:"), retries],
            [label("When a file exists:"), whenExists],
            [label("Server limits:"), NSStackView(views: [hostLimits, forgetLimits])],
            [label("Sign-ins:"), NSStackView(views: [signInsText, manageSignIns])],
            [NSGridCell.emptyContentView, checksumFiles],
            [NSGridCell.emptyContentView, serverDate],
            [NSGridCell.emptyContentView, notify],
        ])
        grid.column(at: 0).xPlacement = .trailing
        grid.rowAlignment = .firstBaseline
        grid.rowSpacing = 10
        grid.columnSpacing = 8
        grid.row(at: 7).topPadding = 6
        grid.translatesAutoresizingMaskIntoConstraints = false
        let content = NSView()
        content.addSubview(grid)
        NSLayoutConstraint.activate([
            grid.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 20),
            grid.trailingAnchor.constraint(lessThanOrEqualTo: content.trailingAnchor, constant: -20),
            grid.topAnchor.constraint(equalTo: content.topAnchor, constant: 20),
            grid.bottomAnchor.constraint(equalTo: content.bottomAnchor, constant: -20),
        ])
        window?.contentView = content
    }

    private func load() {
        let s = AppModel.shared.settings
        folderLabel.stringValue = (AppModel.downloadsFolder.path as NSString).abbreviatingWithTildeInPath
        folderLabel.toolTip = AppModel.downloadsFolder.path
        connections.selectItem(withTitle: "\(s.connections)")
        perServer.selectItem(withTitle: "\(s.connectionsPerServer)")
        atOnce.selectItem(withTitle: "\(s.downloadsAtOnce)")
        retries.selectItem(withTitle: "\(s.retries)")
        speed.limit = s.speedLimit
        whenExists.selectItem(at: Self.existsChoices.firstIndex { $0.1 == s.whenFileExists } ?? 0)
        checksumFiles.state = s.lookForChecksumFiles ? .on : .off
        serverDate.state = s.keepServerDate ? .on : .off
        showHostLimits()
        showSignIns()
        notify.state = s.notifyWhenFinished ? .on : .off
    }

    @objc private func save(_ sender: Any?) {
        var s = AppModel.shared.settings
        let number = { (p: NSPopUpButton, fallback: UInt32) in UInt32(p.titleOfSelectedItem ?? "") ?? fallback }
        s.connections = number(connections, s.connections)
        s.connectionsPerServer = number(perServer, s.connectionsPerServer)
        s.downloadsAtOnce = number(atOnce, s.downloadsAtOnce)
        s.retries = number(retries, s.retries)
        s.speedLimit = speed.limit
        s.whenFileExists = Self.existsChoices[max(0, whenExists.indexOfSelectedItem)].1
        s.lookForChecksumFiles = checksumFiles.state == .on
        s.keepServerDate = serverDate.state == .on
        s.notifyWhenFinished = notify.state == .on
        AppModel.shared.settings = s
        if s.notifyWhenFinished, (sender as? NSButton) === notify { AppModel.shared.askToNotify() }
    }

    /// Servers that turned away extra connections, remembered for a week so later downloads start within the limit.
    private func showHostLimits() {
        let limits = AppModel.shared.manager.hostLimits()
        let text = hostLimitsText(limits: limits)
        hostLimits.stringValue = text.summary
        hostLimits.toolTip = text.detail
        forgetLimits.isEnabled = !limits.isEmpty
    }

    /// "3 saved, encrypted" and the like.
    func showSignIns() {
        let m = AppModel.shared.manager
        let count = m.signIns().count
        let saved = count == 0 ? "None saved" : "\(count) saved"
        let how: String? = switch m.signInStore() {
        case .notChosen: nil
        case .plain: count == 0 ? nil : "as plain text"
        case .locked: "encrypted, locked"
        case .unlocked: "encrypted"
        }
        signInsText.stringValue = [saved, how].compactMap { $0 }.joined(separator: ", ")
        signInsText.toolTip = "Sites' user names and passwords, cookies and referers, for links that need a login"
    }

    @objc private func openSignIns(_ sender: Any?) {
        SignInsWindowController.shared.show()
    }

    @objc private func forgetHostLimits(_ sender: Any?) {
        AppModel.shared.manager.forgetHostLimits()
        showHostLimits()
    }

    @objc private func chooseFolder(_ sender: Any?) {
        guard let window else { return }
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = true
        panel.prompt = "Choose"
        panel.message = "Choose the folder new downloads are saved to"
        panel.directoryURL = AppModel.downloadsFolder
        panel.beginSheetModal(for: window) { [weak self] response in
            guard response == .OK, let url = panel.url else { return }
            MainActor.assumeIsolated {
                var s = AppModel.shared.settings
                s.saveTo = url.path
                AppModel.shared.settings = s
                self?.load()
            }
        }
    }

    @objc private func useDownloads(_ sender: Any?) {
        var s = AppModel.shared.settings
        s.saveTo = nil
        AppModel.shared.settings = s
        load()
    }
}
