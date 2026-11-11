import AppKit
import HDCore

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private(set) var mainWindow: MainWindowController?
    /// Links that arrived (Dock drop, service) before the window was up.
    private var pending: [String] = []

    func applicationDidFinishLaunching(_ notification: Notification) {
        // A click on a "Download finished" notification reaches the app even after it was relaunched.
        if ProcessInfo.processInfo.environment["HD_DATA_DIR"] == nil, Bundle.main.bundleIdentifier != nil {
            FinishedNotifications.shared.register()
        }
        InWindowMenu.install(full: MainMenu.build(), appMenu: MainMenu.appMenu(), appName: "Herb's DM")
        AlertKeys.install()
        let window = MainWindowController()
        mainWindow = window
        // Test hook: --keep-on-top on|off sets Keep on Top at launch (only with $HD_DATA_DIR, so never Herbert's).
        if let i = CommandLine.arguments.firstIndex(of: "--keep-on-top"), i + 1 < CommandLine.arguments.count,
           ProcessInfo.processInfo.environment["HD_DATA_DIR"] != nil {
            KeepOnTop.isOn = CommandLine.arguments[i + 1] == "on"
        }
        KeepOnTop.install()
        if let index = CommandLine.arguments.firstIndex(of: "--menu-report"), index + 1 < CommandLine.arguments.count {
            writeMenuReport(window, to: CommandLine.arguments[index + 1])
            return
        }
        window.showWindow(nil)
        NSApp.servicesProvider = self
        NSUpdateDynamicServices()
        if !pending.isEmpty { window.openLinks(pending); pending = [] }
        // `--add URL` (repeatable) adds and starts a download: the hook the GUI tests use, with HD_DATA_DIR set
        // to a private folder.
        let args = CommandLine.arguments
        // Test hook: --sort-links COLUMN [desc] sorts Add Links as a click on that header does (name, size, folder, link).
        if let i = args.firstIndex(of: "--sort-links"), i + 1 < args.count {
            let descending = args.indices.contains(i + 2) && args[i + 2] == "desc"
            AddLinksWindowController.testSort = NSSortDescriptor(key: args[i + 1], ascending: !descending)
        }
        // Test hooks: --import FILE runs Import Links on a file (§5.7).
        for (i, arg) in args.enumerated() where arg == "--import" && i + 1 < args.count {
            mainWindow?.importLinks(from: [URL(fileURLWithPath: args[i + 1])])
        }
        // Test hooks: --add URL downloads at once, --queue URL adds it to the queue.
        for (i, arg) in args.enumerated() where ["--add", "--queue"].contains(arg) && i + 1 < args.count {
            AppModel.shared.add(url: args[i + 1], start: arg == "--add")
        }
        // Test hook: --settings opens the Settings window (§9).
        if args.contains("--settings") { SettingsWindowController.shared.show() }
        // Test hook: --new-download URL opens New Download with that link (never touching the clipboard).
        if let i = args.firstIndex(of: "--new-download"), i + 1 < args.count { mainWindow?.newDownload(link: args[i + 1]) }
        // Test hook: --download-window [TAB] opens the first download's window, on Info, Connections, Mirrors or Options.
        if let i = args.firstIndex(of: "--download-window"), let row = AppModel.shared.rows.first {
            let tab = args.indices.contains(i + 1) && !args[i + 1].hasPrefix("--") ? args[i + 1] : nil
            let opened = mainWindow?.openDownloadWindow(row.id)
            if let tab { opened?.showTab(tab) }
        }
        // Test hook: --a11y-report FILE opens every window and lists controls without hover text or a VoiceOver name.
        if let i = args.firstIndex(of: "--a11y-report"), i + 1 < args.count, let main = mainWindow {
            let path = args[i + 1]
            SettingsWindowController.shared.show()
            SignInsWindowController.shared.show()
            main.newDownload(nil)
            main.openLinks(["http://127.0.0.1:9/a.bin", "http://127.0.0.1:9/b.bin"])
            main.batchDownload(nil)
            if let row = AppModel.shared.rows.first { _ = main.openDownloadWindow(row.id) }
            main.focusSearch(nil)
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
                let checked = NSApp.windows.filter(\.isVisible).map(\.title).sorted().joined(separator: ", ")
                let lines = ["# checked: \(checked)"] + A11yAudit.problems(in: NSApp.windows)
                try? (lines.joined(separator: "\n") + "\n").write(toFile: path, atomically: true, encoding: .utf8)
                exit(0)
            }
        }
        // Test hook: --features SCRIPT OUT NAME=VALUE… runs the feature tests (FeatureTests.swift).
        FeatureHost.runIfAsked()
        // Test hook: --capture FILE saves the frontmost window (with its title bar) as PNG after 2.5 s, then quits.
        // The timer runs in the common modes, so an open alert (a modal run loop) is captured too.
        if let i = args.firstIndex(of: "--capture"), i + 1 < args.count {
            let path = args[i + 1]
            let timer = Timer(timeInterval: 2.5, repeats: false) { _ in MainActor.assumeIsolated {
                let window = NSApp.modalWindow ?? NSApp.windows.first { $0.isVisible && $0.level == .modalPanel }
                    ?? NSApp.orderedWindows.first { $0.isVisible } ?? NSApp.windows.first
                if let frame = window?.contentView?.superview,
                   let rep = frame.bitmapImageRepForCachingDisplay(in: frame.bounds) {
                    frame.cacheDisplay(in: frame.bounds, to: rep)
                    try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: path))
                }
                exit(0)
            } }
            RunLoop.main.add(timer, forMode: .common)
        }
        NSApp.activate()
    }

    /// Settings… (⌘,) from the app menu, also when no window is open.
    @objc func showSettings(_ sender: Any?) { SettingsWindowController.shared.show() }

    /// Quit while downloads run asks "Downloads are running. Pause them and quit?" with Cancel the default (§10);
    /// quitting pauses them, and they start again at the next launch.
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard AppModel.shared.manager.anyRunning() else { return .terminateNow }
        let alert = NSAlert()
        alert.messageText = "Downloads are running. Pause them and quit?"
        alert.informativeText = "They continue the next time you open Herb's Download Manager."
        alert.addButton(withTitle: "Cancel")
        alert.addButton(withTitle: "Pause and Quit")
        guard alert.runModal() == .alertSecondButtonReturn else { return .terminateCancel }
        _ = AppModel.shared.manager.quit(seconds: 5)
        return .terminateNow
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !flag { mainWindow?.showWindow(nil) }
        return true
    }

    /// Text and HTML files dropped on the Dock icon (§5.4) open Add Links with every link they contain.
    func application(_ application: NSApplication, open urls: [URL]) {
        let pasteboard = NSPasteboard(name: .init("hd-open-\(UUID().uuidString)"))
        pasteboard.clearContents()
        pasteboard.writeObjects(urls as [NSURL])
        openLinks(DropView.links(from: pasteboard))
        pasteboard.releaseGlobally()
    }

    /// The "Download Links with Herb's DM" service, which is also what accepts text and links dropped on the
    /// Dock icon (§5.4).
    @objc func addLinksFromService(_ pasteboard: NSPasteboard, userData: String?,
                                   error: AutoreleasingUnsafeMutablePointer<NSString?>) {
        let links = DropView.links(from: pasteboard)
        guard !links.isEmpty else {
            error.pointee = "No links found" as NSString
            return
        }
        openLinks(links)
    }

    private func openLinks(_ links: [String]) {
        guard !links.isEmpty else { return }
        if mainWindow == nil {
            pending.append(contentsOf: links)
            return
        }
        NSApp.activate()
        mainWindow?.openLinks(links)
    }

    /// Test hook (`--menu-report PATH`, window never shown): where the menus are and a picture of the window with its
    /// title bar, then ⌘Q through the in-window menu, which quits the app when shortcuts reach it.
    private func writeMenuReport(_ window: MainWindowController, to path: String) {
        guard let nsWindow = window.window else { return }
        var lines = ["menu bar: " + (NSApp.mainMenu?.items.map(\.title).joined(separator: ", ") ?? ""),
                     "main menu: " + InWindowMenu.mainMenuTitles.joined(separator: ", ")]
        lines += InWindowMenu.outline()
        if let frame = nsWindow.contentView?.superview, let rep = frame.bitmapImageRepForCachingDisplay(in: frame.bounds) {
            frame.cacheDisplay(in: frame.bounds, to: rep)
            try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: path + ".png"))
        }
        lines.append("routing ⌘Q")
        try? (lines.joined(separator: "\n") + "\n").write(toFile: path, atomically: true, encoding: .utf8)
        DispatchQueue.main.async {
            let quit = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: .command, timestamp: 0,
                                        windowNumber: 0, context: nil, characters: "q", charactersIgnoringModifiers: "q",
                                        isARepeat: false, keyCode: 12)
            if let quit, !InWindowMenu.route(quit) { exit(3) }
        }
    }

    static let projectURL = URL(string: "https://github.com/dunkelherzmann/herbs-download-manager")!

    /// Help ▸ Herb's Download Manager Help (⌘?): the guide, HELP.md, on the project's page.
    @objc func openHelp(_ sender: Any?) {
        NSWorkspace.shared.open(Self.projectURL.appendingPathComponent("blob/main/HELP.md"))
    }

    /// About: the name, version, what it is, the author and a link to the project.
    @objc func showAbout(_ sender: Any?) {
        let appVersion = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "dev"
        let centered = NSMutableParagraphStyle()
        centered.alignment = .center
        let base: [NSAttributedString.Key: Any] = [.font: NSFont.systemFont(ofSize: 11), .foregroundColor: NSColor.labelColor,
                                                   .paragraphStyle: centered]
        let credits = NSMutableAttributedString(string: "A download manager\nMany connections · Mirrors · Streams\n\nBy dunkelherzmann\n",
                                                attributes: base)
        var link = base
        link[.link] = Self.projectURL
        credits.append(NSAttributedString(string: Self.projectURL.absoluteString, attributes: link))
        NSApp.orderFrontStandardAboutPanel(options: [
            .applicationName: "Herb's Download Manager",
            .applicationVersion: appVersion,
            .version: "core \(coreVersion())",
            .credits: credits,
        ])
    }
}
