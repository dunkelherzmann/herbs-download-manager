import AppKit

/// Keyboard choice in alerts and sheets (SPECIFICATION.md §4): the arrow keys and Tab move the highlight (the blue
/// button) from one button to the next, Return or Enter presses the highlighted one, and Esc cancels, whether or not
/// Keyboard navigation is on in System Settings. A text field in the alert keeps Tab and the arrows for itself.
@MainActor
enum AlertKeys {
    /// Each button's own key equivalent ("\r" for the default, Esc for Cancel), kept while the highlight moves.
    private static var own: [ObjectIdentifier: String] = [:]

    static func install() {
        NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            MainActor.assumeIsolated { handle(event) } ? nil : event
        }
    }

    /// True when the key was taken here.
    private static func handle(_ event: NSEvent) -> Bool {
        guard let window = NSApp.keyWindow, window.isSheet || NSApp.modalWindow === window else { return false }
        // Typing in a field: its keys are its own.
        if window.firstResponder is NSText { return false }
        let flags = event.modifierFlags.intersection([.command, .option, .control])
        guard flags.isEmpty else { return false }
        let buttons = pushButtons(in: window)
        guard buttons.count > 1 else { return false }
        for b in buttons where own[ObjectIdentifier(b)] == nil {
            own[ObjectIdentifier(b)] = b.keyEquivalent
        }
        let current = buttons.firstIndex { $0.keyEquivalent == "\r" }
        let step: Int
        switch event.keyCode {
        case 125, 124: step = 1                                        // ↓ →
        case 126, 123: step = -1                                       // ↑ ←
        case 48: step = event.modifierFlags.contains(.shift) ? -1 : 1  // Tab, ⇧Tab
        case 53:                                                       // Esc: the button that cancels
            guard let cancel = buttons.first(where: { own[ObjectIdentifier($0)] == "\u{1b}" }) else { return false }
            cancel.performClick(nil)
            return true
        default:
            return false
        }
        let next = current.map { ($0 + step + buttons.count) % buttons.count } ?? (step > 0 ? 0 : buttons.count - 1)
        highlight(buttons[next], among: buttons)
        return true
    }

    /// Makes `button` the one Return presses (drawn blue); the others get back their own keys, but not Return.
    private static func highlight(_ button: NSButton, among buttons: [NSButton]) {
        for b in buttons where b !== button {
            let key = own[ObjectIdentifier(b)] ?? ""
            b.keyEquivalent = key == "\r" ? "" : key
        }
        button.keyEquivalent = "\r"
        button.window?.recalculateKeyViewLoop()
        NSAccessibility.post(element: button, notification: .focusedUIElementChanged)
    }

    /// The window's enabled push buttons, top to bottom, then left to right, as they're read.
    private static func pushButtons(in window: NSWindow) -> [NSButton] {
        var found: [NSButton] = []
        func walk(_ view: NSView) {
            if let b = view as? NSButton, b.bezelStyle == .push, b.isEnabled, !b.isHiddenOrHasHiddenAncestor, !b.title.isEmpty {
                found.append(b)
            }
            view.subviews.forEach(walk)
        }
        if let content = window.contentView { walk(content) }
        return found.sorted { a, b in
            let (fa, fb) = (a.convert(a.bounds, to: nil), b.convert(b.bounds, to: nil))
            // Window coordinates grow upward: higher first, then leftmost.
            return abs(fa.midY - fb.midY) > 4 ? fa.midY > fb.midY : fa.minX < fb.minX
        }
    }
}
