import AppKit

/// The app menu and Window menu. The full menu bar (SPECIFICATION.md §4) comes with the main window.
@MainActor
enum MainMenu {
    static func build() -> NSMenu {
        let bar = NSMenu()

        let appItem = NSMenuItem()
        let appMenu = NSMenu(title: "Herb's Downloader")
        appMenu.addItem(withTitle: "About Herb's Downloader", action: #selector(AppDelegate.showAbout(_:)), keyEquivalent: "")
        appMenu.addItem(.separator())
        appMenu.addItem(withTitle: "Hide Herb's Downloader", action: #selector(NSApplication.hide(_:)), keyEquivalent: "h")
        let others = appMenu.addItem(withTitle: "Hide Others", action: #selector(NSApplication.hideOtherApplications(_:)), keyEquivalent: "h")
        others.keyEquivalentModifierMask = [.command, .option]
        appMenu.addItem(withTitle: "Show All", action: #selector(NSApplication.unhideAllApplications(_:)), keyEquivalent: "")
        appMenu.addItem(.separator())
        appMenu.addItem(withTitle: "Quit Herb's Downloader", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        appItem.submenu = appMenu
        bar.addItem(appItem)

        let windowItem = NSMenuItem()
        let windowMenu = NSMenu(title: "Window")
        windowMenu.addItem(withTitle: "Close", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")
        windowMenu.addItem(withTitle: "Minimize", action: #selector(NSWindow.performMiniaturize(_:)), keyEquivalent: "m")
        windowMenu.addItem(withTitle: "Zoom", action: #selector(NSWindow.performZoom(_:)), keyEquivalent: "")
        windowItem.submenu = windowMenu
        bar.addItem(windowItem)
        NSApp.windowsMenu = windowMenu

        return bar
    }
}
