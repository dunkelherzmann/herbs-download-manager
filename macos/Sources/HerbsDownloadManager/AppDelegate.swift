import AppKit
import HDCore

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var mainWindow: MainWindowController?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.mainMenu = MainMenu.build()
        let window = MainWindowController()
        window.showWindow(nil)
        mainWindow = window
        // `--add URL` (repeatable) adds and starts a download: the hook the GUI tests use, with HD_DATA_DIR set
        // to a private folder.
        let args = CommandLine.arguments
        for (i, arg) in args.enumerated() where arg == "--add" && i + 1 < args.count {
            AppModel.shared.add(url: args[i + 1], start: true)
        }
        NSApp.activate()
    }

    /// Running downloads are paused (their resume state saved) before the app quits; the quit question (§10)
    /// comes with pause and resume in the app.
    func applicationWillTerminate(_ notification: Notification) {
        AppModel.shared.pauseAll()
        _ = AppModel.shared.manager.waitIdle(seconds: 5)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !flag { mainWindow?.showWindow(nil) }
        return true
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
