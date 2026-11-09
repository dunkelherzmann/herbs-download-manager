import AppKit

/// Every menu lives in the window, not in the macOS menu bar (SPECIFICATION.md §2, §4): wide screens make the trip
/// to the top of the screen long. The menu bar keeps only the app menu macOS always shows; the toolbar's Main Menu
/// (☰) holds the rest, as in Herb's Terminal, and its shortcuts work from any window.
@MainActor
enum InWindowMenu {
    /// The Main Menu: a submenu per menu, then Settings, About, Hide and Quit.
    private(set) static var menu = NSMenu()
    private static var monitor: Any?

    /// Shows only `appMenu` in the macOS menu bar and routes key equivalents to `full`.
    static func install(full: NSMenu, appMenu: NSMenu, appName: String) {
        menu = full
        let bar = NSMenu()
        bar.addItem(withTitle: appName, action: nil, keyEquivalent: "").submenu = appMenu
        NSApp.mainMenu = bar
        if monitor == nil {
            monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
                route(event) ? nil : event
            }
        }
    }

    /// AppKit's own order: the key window (its fields, buttons and sheets) first, then the in-window menu, which
    /// takes the place of the menu bar. Returns whether the event was handled.
    static func route(_ event: NSEvent) -> Bool {
        let modifiers = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        guard modifiers.contains(.command) || modifiers.contains(.control) else { return false }
        if let window = NSApp.keyWindow, window.performKeyEquivalent(with: event) { return true }
        return menu.performKeyEquivalent(with: event)
    }

    /// The Main Menu button (☰) at the right end of the toolbar, after Search, as in Herb's Terminal: every menu
    /// in one place, so the title bar holds only the window controls, the title and the toolbar.
    static func toolbarItem(_ id: NSToolbarItem.Identifier) -> NSToolbarItem {
        let item = NSMenuToolbarItem(itemIdentifier: id)
        item.menu = menu
        item.showsIndicator = false
        item.image = NSImage(systemSymbolName: "line.3.horizontal", accessibilityDescription: "Main Menu")
        item.label = "Main Menu"
        item.paletteLabel = "Main Menu"
        item.toolTip = "Main Menu: every command, from File, Edit, Tasks, View, Window and Help to Settings and Quit"
        item.isBordered = true
        return item
    }

    /// The Main Menu's top level (for the tests).
    static var mainMenuTitles: [String] {
        menu.items.filter { !$0.isSeparatorItem }.map(\.title)
    }

    /// The menu tree as indented lines, for the tests.
    static func outline(_ menu: NSMenu? = nil, depth: Int = 0) -> [String] {
        (menu ?? self.menu).items.flatMap { item -> [String] in
            guard !item.isSeparatorItem else { return [] }
            let line = String(repeating: "  ", count: depth) + item.title
            return [line] + (item.submenu.map { outline($0, depth: depth + 1) } ?? [])
        }
    }
}
