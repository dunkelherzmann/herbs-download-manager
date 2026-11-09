import AppKit

/// Every control explains itself (Herbert's rule; §3, §4): a hover bubble saying what it is and does, and a name for
/// VoiceOver. `--a11y-report FILE` opens each window and writes one line per interactive control that lacks either;
/// scripts/test.sh fails when any line is written.
@MainActor
enum A11yAudit {
    static func problems(in windows: [NSWindow]) -> [String] {
        var out: [String] = []
        for window in windows where window.isVisible {
            let place = window.title.isEmpty ? String(describing: type(of: window)) : window.title
            for item in window.toolbar?.items ?? [] where item.view == nil || item.view is NSButton {
                let id = item.itemIdentifier.rawValue
                if id.hasPrefix("sep") || id.hasPrefix("NSToolbar") { continue }
                if (item.toolTip ?? "").isEmpty { out.append("\(place) | toolbar \(id) | no hover text") }
                if item.label.isEmpty && (item.image?.accessibilityDescription ?? "").isEmpty {
                    out.append("\(place) | toolbar \(id) | no name")
                }
            }
            if let content = window.contentView { walk(content, place: place, into: &out) }
        }
        return out
    }

    private static func walk(_ view: NSView, place: String, into out: inout [String]) {
        if view.isHidden { return }
        // System parts VoiceOver names itself: rows' controls in tables and outlines (disclosure triangles, row
        // checkboxes, which carry the row's own label) and a search field's search and cancel buttons.
        if view is NSTableView || view is NSOutlineView || view is NSSearchField {
            if let field = view as? NSSearchField, (field.toolTip ?? "").isEmpty {
                out.append("\(place) | NSSearchField “\(field.placeholderString ?? "")” | no hover text")
            }
            return
        }
        // Table and outline rows are data, read by VoiceOver from their cells; their own controls are checked here.
        if let control = view as? NSControl, isInteractive(control) {
            let name = describe(control)
            if (control.toolTip ?? "").isEmpty { out.append("\(place) | \(name) | no hover text") }
            if !hasName(control) { out.append("\(place) | \(name) | no VoiceOver name") }
        }
        for sub in view.subviews { walk(sub, place: place, into: &out) }
    }

    private static func isInteractive(_ c: NSControl) -> Bool {
        if c is NSTableView || c is NSOutlineView { return false }
        if let field = c as? NSTextField { return field.isEditable }
        return c is NSButton || c is NSSegmentedControl || c is NSSlider || c is NSStepper || c is NSPathControl
    }

    private static func hasName(_ c: NSControl) -> Bool {
        if !(c.accessibilityLabel() ?? "").isEmpty || c.accessibilityTitleUIElement() != nil { return true }
        // A popup's shown value isn't its name ("32" says nothing): it needs a label of its own.
        if c is NSPopUpButton { return false }
        if let button = c as? NSButton { return !button.title.isEmpty || !(button.image?.accessibilityDescription ?? "").isEmpty }
        if let field = c as? NSTextField { return !(field.placeholderString ?? "").isEmpty }
        return false
    }

    private static func describe(_ c: NSControl) -> String {
        let kind = String(describing: type(of: c))
        let text = (c as? NSButton)?.title ?? (c as? NSTextField)?.placeholderString ?? ""
        let id = c.accessibilityIdentifier()
        return [kind, text.isEmpty ? nil : "“\(text)”", id.isEmpty ? nil : "#\(id)"].compactMap { $0 }.joined(separator: " ")
    }
}
