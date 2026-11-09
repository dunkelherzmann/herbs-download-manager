import AppKit
import HDCore

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private(set) var mainWindow: MainWindowController?
    /// Links that arrived (Dock drop, service) before the window was up.
    private var pending: [String] = []

    func applicationDidFinishLaunching(_ notification: Notification) {
        InWindowMenu.install(full: MainMenu.build(), appMenu: MainMenu.appMenu(), appName: "Herb's DM")
        let window = MainWindowController()
        InWindowMenu.attach(to: window.window!)
        mainWindow = window
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
        // Test hooks: --import FILE runs Import Links on a file (§5.7).
        for (i, arg) in args.enumerated() where arg == "--import" && i + 1 < args.count {
            mainWindow?.importLinks(from: [URL(fileURLWithPath: args[i + 1])])
        }
        // Test hooks: --add URL downloads at once, --queue URL adds it to the queue.
        for (i, arg) in args.enumerated() where ["--add", "--queue"].contains(arg) && i + 1 < args.count {
            AppModel.shared.add(url: args[i + 1], start: arg == "--add")
        }
        NSApp.activate()
    }

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
                     "title bar menus: " + InWindowMenu.barTitles(in: nsWindow).joined(separator: ", ")]
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

    @objc func openGitHub(_ sender: Any?) {
        NSWorkspace.shared.open(URL(string: "https://github.com/dunkelherzmann/herbs-download-manager")!)
    }

    @objc func showAbout(_ sender: Any?) {
        let appVersion = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "dev"
        NSApp.orderFrontStandardAboutPanel(options: [
            .applicationName: "Herb's Download Manager",
            .applicationVersion: appVersion,
            .version: "core \(coreVersion())",
        ])
    }
}
