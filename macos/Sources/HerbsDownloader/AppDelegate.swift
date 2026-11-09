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
        NSApp.activate()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !flag { mainWindow?.showWindow(nil) }
        return true
    }

    @objc func showAbout(_ sender: Any?) {
        let appVersion = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "dev"
        NSApp.orderFrontStandardAboutPanel(options: [
            .applicationName: "Herb's Downloader",
            .applicationVersion: appVersion,
            .version: "core \(coreVersion())",
        ])
    }
}
