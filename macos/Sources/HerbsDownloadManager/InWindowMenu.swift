import AppKit

/// Every menu lives in the window, not in the macOS menu bar (SPECIFICATION.md §2, §4): wide screens make the trip
/// to the top of the screen long. The menu bar keeps only the app menu macOS always shows; the window's title bar
/// holds a menu bar of its own, as on Linux and Windows, and its shortcuts work from any window.
@MainActor
enum InWindowMenu {
    /// The full menu: one submenu per menu of the title bar's menu bar.
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

    /// Puts the menu bar (one pull-down per menu) at the left of the window's title bar, after the window controls.
    static func attach(to window: NSWindow) {
        let stack = NSStackView()
        stack.orientation = .horizontal
        stack.spacing = 0
        stack.setAccessibilityRole(.menuBar)
        stack.setAccessibilityLabel("Menu bar")
        for item in menu.items {
            guard let submenu = item.submenu else { continue }
            let button = MenuTitleButton(title: item.title, menu: submenu)
            stack.addArrangedSubview(button)
        }
        let size = stack.fittingSize
        // As tall as the title bar (with the unified toolbar), so the menus line up with the window controls.
        let height = max(28, window.frame.height - window.contentLayoutRect.height)
        let holder = NSView(frame: NSRect(x: 0, y: 0, width: size.width + 8, height: height))
        stack.frame = NSRect(x: 4, y: ((height - size.height) / 2).rounded(), width: size.width, height: size.height)
        holder.addSubview(stack)
        let accessory = NSTitlebarAccessoryViewController()
        accessory.view = holder
        accessory.layoutAttribute = .leading
        window.addTitlebarAccessoryViewController(accessory)
    }

    /// The titles of the menu bar in `window`, if it has one (for the tests).
    static func barTitles(in window: NSWindow) -> [String] {
        window.titlebarAccessoryViewControllers.flatMap { controller in
            controller.view.subviews.compactMap { $0 as? NSStackView }.flatMap { stack in
                stack.arrangedSubviews.compactMap { ($0 as? MenuTitleButton)?.title }
            }
        }
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

/// One menu of the title bar's menu bar: its title, opening its menu below it.
@MainActor
private final class MenuTitleButton: NSButton {
    private let pulldown: NSMenu

    init(title: String, menu: NSMenu) {
        pulldown = menu
        super.init(frame: .zero)
        self.title = title
        isBordered = false
        bezelStyle = .accessoryBarAction
        showsBorderOnlyWhileMouseInside = true
        font = .menuBarFont(ofSize: 0)
        toolTip = "\(title) menu"
        setAccessibilityRole(.menuBarItem)
        setAccessibilityLabel(title)
        target = self
        action = #selector(open)
        setContentHuggingPriority(.required, for: .horizontal)
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    override var intrinsicContentSize: NSSize {
        let base = super.intrinsicContentSize
        return NSSize(width: base.width + 10, height: max(base.height, 22))
    }

    @objc private func open() {
        pulldown.popUp(positioning: nil, at: NSPoint(x: 0, y: bounds.height + 4), in: self)
    }
}
