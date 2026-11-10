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

    private let startOn = NSButton(checkboxWithTitle: "Start the queue at", target: nil, action: nil)
    private let startTime = NSDatePicker()
    private let stopOn = NSButton(checkboxWithTitle: "Stop the queue at", target: nil, action: nil)
    private let stopTime = NSDatePicker()
    private let days = (0..<7).map { _ in NSButton(checkboxWithTitle: "", target: nil, action: nil) }
    private let whenDone = NSPopUpButton()
    private let streamQuality = NSPopUpButton()
    private let soundLanguage = NSPopUpButton()
    private let subtitles = NSPopUpButton()
    private static let qualities: [UInt32?] = [nil] + streamQualityChoices().map { Optional($0) }
    private static let languages = soundLanguageChoices()
    private let showExtension = NSButton(title: "Show Extension Folder", target: nil, action: nil)

    private static let doneChoices: [(String, WhenDone)] = [
        ("Do nothing", .nothing), ("Quit", .quit), ("Sleep", .sleep), ("Shut down", .shutDown),
    ]

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

        buildScheduler()
        buildStreams()
        showExtension.target = self
        showExtension.action = #selector(revealExtension(_:))
        showExtension.toolTip = "Show the extension that hands Chrome's, Edge's, Brave's, Vivaldi's and Firefox's downloads to the app"
        showExtension.setAccessibilityIdentifier("settings-show-extension")
        let extensionHelp = NSTextField(wrappingLabelWithString: "Chrome, Edge, Brave and Vivaldi: Extensions ▸ Developer mode ▸ "
            + "Load Unpacked, and choose the folder. Firefox: about:debugging ▸ This Firefox ▸ Load Temporary Add-on, and choose "
            + "manifest.json in it. Safari isn't supported yet.")
        extensionHelp.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        extensionHelp.textColor = .secondaryLabelColor
        extensionHelp.preferredMaxLayoutWidth = 330

        let label = { (text: String) in NSTextField(labelWithString: text) }
        let grid = NSGridView(views: [
            [label("Save downloads to:"), folderRow],
            [label("Connections per download:"), connections],
            [label("Connections per server:"), perServer],
            [label("Downloads at once:"), atOnce],
            [label("Speed limit:"), speed],
            [label("Retries per source:"), retries],
            [label("When a file exists:"), whenExists],
            [label("Scheduler:"), NSStackView(views: [startOn, startTime])],
            [NSGridCell.emptyContentView, NSStackView(views: [stopOn, stopTime])],
            [label("On:"), NSStackView(views: days)],
            [label("When the queue is done:"), whenDone],
            [label("Video stream quality:"), streamQuality],
            [label("Sound language:"), soundLanguage],
            [label("Subtitles:"), subtitles],
            [label("Server limits:"), NSStackView(views: [hostLimits, forgetLimits])],
            [label("Sign-ins:"), NSStackView(views: [signInsText, manageSignIns])],
            [label("Browser extension:"), showExtension],
            [NSGridCell.emptyContentView, extensionHelp],
            [NSGridCell.emptyContentView, checksumFiles],
            [NSGridCell.emptyContentView, serverDate],
            [NSGridCell.emptyContentView, notify],
        ])
        grid.column(at: 0).xPlacement = .trailing
        grid.rowAlignment = .firstBaseline
        grid.rowSpacing = 10
        grid.columnSpacing = 8
        grid.row(at: 7).topPadding = 6
        grid.row(at: 11).topPadding = 6
        grid.row(at: 14).topPadding = 6
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
        startOn.state = s.scheduleStart == nil ? .off : .on
        stopOn.state = s.scheduleStop == nil ? .off : .on
        startTime.dateValue = Self.date(s.scheduleStart ?? 60)
        stopTime.dateValue = Self.date(s.scheduleStop ?? 7 * 60)
        for (box, on) in zip(days, s.scheduleDays) { box.state = on ? .on : .off }
        whenDone.selectItem(at: Self.doneChoices.firstIndex { $0.1 == s.whenDone } ?? 0)
        streamQuality.selectItem(at: Self.qualities.firstIndex { $0 == s.streamQuality } ?? 0)
        // The first preferred language; one not offered here (a hand-edited settings file) is shown as its code.
        if let first = s.soundLanguages.first {
            if !Self.languages.contains(where: { $0.code == first }), soundLanguage.numberOfItems == Self.languages.count + 1 {
                soundLanguage.addItem(withTitle: first)
            }
            let i = Self.languages.firstIndex { $0.code == first }.map { $0 + 1 } ?? soundLanguage.numberOfItems - 1
            soundLanguage.selectItem(at: i)
        } else {
            soundLanguage.selectItem(at: 0)
        }
        // S15: None, All, or a language (one not offered here shown as its code).
        switch s.subtitleLanguages.first {
        case nil: subtitles.selectItem(at: 0)
        case "*": subtitles.selectItem(at: 1)
        case let code?:
            if !Self.languages.contains(where: { $0.code == code }), subtitles.numberOfItems == Self.languages.count + 2 {
                subtitles.addItem(withTitle: code)
            }
            subtitles.selectItem(at: Self.languages.firstIndex { $0.code == code }.map { $0 + 2 } ?? subtitles.numberOfItems - 1)
        }
        enableScheduler()
    }

    /// S13, S14 and S15 (§15): what a stream added from now on downloads, unless the extension chose for it.
    private func buildStreams() {
        streamQuality.addItems(withTitles: Self.qualities.map { $0.map { "\($0)p" } ?? "Highest" })
        streamQuality.target = self
        streamQuality.action = #selector(save(_:))
        streamQuality.toolTip = "The quality a streamed video is downloaded in: the best one up to this size, or the smallest "
            + "when every one is bigger. The browser extension can choose another for each video"
        streamQuality.setAccessibilityLabel("Video stream quality")
        streamQuality.setAccessibilityIdentifier("settings-stream-quality")
        soundLanguage.addItems(withTitles: ["The stream's own"] + Self.languages.map(\.name))
        soundLanguage.target = self
        soundLanguage.action = #selector(save(_:))
        soundLanguage.toolTip = "For a stream with sound in several languages: the one to download. When the stream doesn't "
            + "have it, its own sound is taken"
        soundLanguage.setAccessibilityLabel("Sound language")
        soundLanguage.setAccessibilityIdentifier("settings-sound-language")
        subtitles.addItems(withTitles: ["None", "All"] + Self.languages.map(\.name))
        subtitles.target = self
        subtitles.action = #selector(save(_:))
        subtitles.toolTip = "Subtitles a streamed video's file keeps, as tracks a player can turn on: none, every language "
            + "the stream has, or one language"
        subtitles.setAccessibilityLabel("Subtitles")
        subtitles.setAccessibilityIdentifier("settings-subtitles")
    }

    /// Q1–Q4 (§11.1). The days run Monday first, named as the Mac's language names them.
    private func buildScheduler() {
        for (box, picker, what) in [(startOn, startTime, "start"), (stopOn, stopTime, "stop")] {
            box.target = self
            box.action = #selector(save(_:))
            box.setAccessibilityIdentifier("settings-schedule-\(what)")
            picker.datePickerElements = .hourMinute
            picker.datePickerStyle = .textFieldAndStepper
            picker.target = self
            picker.action = #selector(save(_:))
            picker.setAccessibilityLabel("Time to \(what) the queue")
            picker.setAccessibilityIdentifier("settings-schedule-\(what)-time")
        }
        startOn.toolTip = "Start the queue at this time on the days chosen below"
        stopOn.toolTip = "Stop the queue at this time; downloads still running pause and wait in the queue for the next start. "
            + "A time before the start time is the next morning's"
        let short = Calendar.current.shortWeekdaySymbols  // Sunday first
        let full = Calendar.current.weekdaySymbols
        for (i, box) in days.enumerated() {
            box.title = short[(i + 1) % 7]
            box.target = self
            box.action = #selector(save(_:))
            box.toolTip = "Start the queue on \(full[(i + 1) % 7])s"
            box.setAccessibilityIdentifier("settings-schedule-day-\(i)")
        }
        whenDone.addItems(withTitles: Self.doneChoices.map(\.0))
        whenDone.target = self
        whenDone.action = #selector(save(_:))
        whenDone.toolTip = "Once the running queue has nothing left to download: quit the app, or put the Mac to sleep or "
            + "shut it down, after a minute's countdown you can cancel"
        whenDone.setAccessibilityLabel("When the queue is done")
        whenDone.setAccessibilityIdentifier("settings-when-done")
    }

    private func enableScheduler() {
        startTime.isEnabled = startOn.state == .on
        stopTime.isEnabled = stopOn.state == .on
        for box in days { box.isEnabled = startOn.state == .on || stopOn.state == .on }
    }

    private static func date(_ minutes: UInt32) -> Date {
        Calendar.current.date(bySettingHour: Int(minutes / 60), minute: Int(minutes % 60), second: 0, of: Date()) ?? Date()
    }

    private static func minutes(_ date: Date) -> UInt32 {
        let c = Calendar.current.dateComponents([.hour, .minute], from: date)
        return UInt32((c.hour ?? 0) * 60 + (c.minute ?? 0))
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
        s.scheduleStart = startOn.state == .on ? Self.minutes(startTime.dateValue) : nil
        s.scheduleStop = stopOn.state == .on ? Self.minutes(stopTime.dateValue) : nil
        s.scheduleDays = days.map { $0.state == .on }
        s.whenDone = Self.doneChoices[max(0, whenDone.indexOfSelectedItem)].1
        s.streamQuality = Self.qualities[max(0, streamQuality.indexOfSelectedItem)]
        // Only a change here replaces the list, so a hand-edited list of several languages survives other changes.
        if (sender as? NSPopUpButton) === soundLanguage {
            let i = soundLanguage.indexOfSelectedItem
            s.soundLanguages = i <= 0 ? [] : i <= Self.languages.count ? [Self.languages[i - 1].code] : s.soundLanguages
        }
        if (sender as? NSPopUpButton) === subtitles {
            let i = subtitles.indexOfSelectedItem
            s.subtitleLanguages = i <= 0 ? [] : i == 1 ? ["*"] : i <= Self.languages.count + 1 ? [Self.languages[i - 2].code] : s.subtitleLanguages
        }
        enableScheduler()
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

    /// The extension in the app's data folder, shown in Finder, for Load Unpacked.
    @objc private func revealExtension(_ sender: Any?) {
        do {
            let folder = URL(fileURLWithPath: try AppModel.shared.manager.extensionFolder(), isDirectory: true)
            NSWorkspace.shared.activateFileViewerSelecting([folder])
        } catch {
            let alert = NSAlert()
            alert.messageText = "The extension can't be written to the app's folder"
            alert.informativeText = errorText(error)
            if let window { alert.beginSheetModal(for: window) } else { alert.runModal() }
        }
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
