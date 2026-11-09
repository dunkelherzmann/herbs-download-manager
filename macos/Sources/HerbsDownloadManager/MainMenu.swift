import AppKit

/// The menus (SPECIFICATION.md §4): the app menu, File, Edit, Tasks, View, Window and Help, shown in the main
/// window's title bar (InWindowMenu). Commands that act on downloads go to the main window's controller through the
/// responder chain, which also enables them.
@MainActor
enum MainMenu {
    static func build() -> NSMenu {
        let bar = NSMenu()

        let file = submenu(bar, "File")
        file.addItem(withTitle: "New Download…", action: #selector(MainWindowController.newDownload(_:)), keyEquivalent: "n")
        item(file, "Import Links…", #selector(MainWindowController.importLinks(_:)), "o", [.command, .shift])
        item(file, "Add from Clipboard", #selector(MainWindowController.addFromClipboard(_:)), "v", [.command, .shift])
        item(file, "Batch Download…", #selector(MainWindowController.batchDownload(_:)), "n", [.command, .shift])
        file.addItem(.separator())
        file.addItem(withTitle: "Close Window", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")

        let edit = submenu(bar, "Edit")
        edit.addItem(withTitle: "Undo", action: Selector(("undo:")), keyEquivalent: "z")
        item(edit, "Redo", Selector(("redo:")), "z", [.command, .shift])
        edit.addItem(.separator())
        edit.addItem(withTitle: "Cut", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        edit.addItem(withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        edit.addItem(withTitle: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        edit.addItem(withTitle: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
        edit.addItem(.separator())
        edit.addItem(withTitle: "Find", action: #selector(MainWindowController.focusSearch(_:)), keyEquivalent: "f")

        let tasks = submenu(bar, "Tasks")
        tasks.addItem(withTitle: "Resume", action: #selector(MainWindowController.resumeSelected(_:)), keyEquivalent: "r")
        tasks.addItem(withTitle: "Pause", action: #selector(MainWindowController.pauseSelected(_:)), keyEquivalent: ".")
        tasks.addItem(.separator())
        item(tasks, "Start Queue", #selector(MainWindowController.startQueue(_:)), "r", [.command, .shift])
        tasks.addItem(withTitle: "Stop Queue", action: #selector(MainWindowController.stopQueue(_:)), keyEquivalent: "")
        tasks.addItem(.separator())
        item(tasks, "Stop All", #selector(MainWindowController.stopAll(_:)), ".", [.command, .option])
        tasks.addItem(.separator())
        item(tasks, "Add Mirror…", #selector(MainWindowController.addMirror(_:)), "m", [.command, .option])
        tasks.addItem(withTitle: "Properties", action: #selector(MainWindowController.showProperties(_:)), keyEquivalent: "i")
        tasks.addItem(.separator())
        tasks.addItem(withTitle: "Rename…", action: #selector(MainWindowController.renameFile(_:)), keyEquivalent: "")
        tasks.addItem(withTitle: "Move To…", action: #selector(MainWindowController.moveFile(_:)), keyEquivalent: "")
        tasks.addItem(.separator())
        let deleteItem = tasks.addItem(withTitle: "Delete", action: nil, keyEquivalent: "")
        let delete = NSMenu(title: "Delete")
        item(delete, "Selected", #selector(MainWindowController.deleteSelected(_:)), "\u{8}", [.command])
        delete.addItem(withTitle: "All Finished", action: #selector(MainWindowController.deleteAllFinished(_:)), keyEquivalent: "")
        delete.addItem(withTitle: "All Unfinished", action: #selector(MainWindowController.deleteAllUnfinished(_:)), keyEquivalent: "")
        delete.addItem(withTitle: "All Missing Files", action: #selector(MainWindowController.deleteMissingFiles(_:)), keyEquivalent: "")
        delete.addItem(withTitle: "Entire List", action: #selector(MainWindowController.deleteEntireList(_:)), keyEquivalent: "")
        deleteItem.submenu = delete

        let view = submenu(bar, "View")
        item(view, "Show Sidebar", #selector(MainWindowController.toggleSidebar(_:)), "s", [.command, .control])
        item(view, "Keep on Top", #selector(MainWindowController.toggleKeepOnTop(_:)), "t", [.command, .control])
        let columnsItem = view.addItem(withTitle: "Columns", action: nil, keyEquivalent: "")
        let columns = NSMenu(title: "Columns")
        columns.delegate = ColumnsMenu.shared
        columnsItem.submenu = columns
        view.addItem(withTitle: "Show Download Window", action: #selector(MainWindowController.showDownloadWindow(_:)), keyEquivalent: "o")

        let window = submenu(bar, "Window")
        window.addItem(withTitle: "Minimize", action: #selector(NSWindow.performMiniaturize(_:)), keyEquivalent: "m")
        window.addItem(withTitle: "Zoom", action: #selector(NSWindow.performZoom(_:)), keyEquivalent: "")
        window.addItem(.separator())
        window.addItem(withTitle: "Bring All to Front", action: #selector(NSApplication.arrangeInFront(_:)), keyEquivalent: "")
        NSApp.windowsMenu = window

        let help = submenu(bar, "Help")
        help.addItem(withTitle: "Herb's Download Manager on GitHub", action: #selector(AppDelegate.openGitHub(_:)), keyEquivalent: "")

        // The app menu's commands close the Main Menu, as in Herb's Terminal.
        bar.addItem(.separator())
        bar.addItem(withTitle: "Settings…", action: #selector(MainWindowController.showSettings(_:)), keyEquivalent: ",")
        bar.addItem(withTitle: "About Herb's Download Manager", action: #selector(AppDelegate.showAbout(_:)), keyEquivalent: "")
        bar.addItem(.separator())
        bar.addItem(withTitle: "Hide Herb's DM", action: #selector(NSApplication.hide(_:)), keyEquivalent: "h")
        bar.addItem(withTitle: "Quit Herb's DM", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        return bar
    }

    /// The app menu, under the short name: in the macOS menu bar (the one menu macOS always shows there) and first in
    /// the title bar's menu bar.
    static func appMenu() -> NSMenu {
        let app = NSMenu(title: "Herb's DM")
        app.addItem(withTitle: "About Herb's Download Manager", action: #selector(AppDelegate.showAbout(_:)), keyEquivalent: "")
        app.addItem(.separator())
        app.addItem(withTitle: "Settings…", action: #selector(MainWindowController.showSettings(_:)), keyEquivalent: ",")
        app.addItem(.separator())
        app.addItem(withTitle: "Hide Herb's DM", action: #selector(NSApplication.hide(_:)), keyEquivalent: "h")
        item(app, "Hide Others", #selector(NSApplication.hideOtherApplications(_:)), "h", [.command, .option])
        app.addItem(withTitle: "Show All", action: #selector(NSApplication.unhideAllApplications(_:)), keyEquivalent: "")
        app.addItem(.separator())
        app.addItem(withTitle: "Quit Herb's DM", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        return app
    }

    private static func submenu(_ bar: NSMenu, _ title: String) -> NSMenu {
        let holder = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        let menu = NSMenu(title: title)
        holder.submenu = menu
        bar.addItem(holder)
        return menu
    }

    @discardableResult
    private static func item(_ menu: NSMenu, _ title: String, _ action: Selector, _ key: String,
                             _ modifiers: NSEvent.ModifierFlags) -> NSMenuItem {
        let item = menu.addItem(withTitle: title, action: action, keyEquivalent: key)
        item.keyEquivalentModifierMask = modifiers
        return item
    }
}

/// View ▸ Columns: one item per column but Name, ticked when shown.
@MainActor
final class ColumnsMenu: NSObject, NSMenuDelegate {
    static let shared = ColumnsMenu()

    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        guard let list = (NSApp.delegate as? AppDelegate)?.mainWindow?.list else { return }
        list.fillColumnsMenu(menu)
    }
}
