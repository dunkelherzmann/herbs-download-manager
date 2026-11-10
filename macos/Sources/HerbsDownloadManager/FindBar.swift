import AppKit

/// The find bar (§2, T10), as in Herb's Terminal: the toolbar's 🔍 or ⌘F shows it at the top right of the list,
/// it filters the list as you type (name, link or note), and Done or Esc hides it and shows every download again.
@MainActor
final class FindBar: NSVisualEffectView, NSSearchFieldDelegate {
    let field = NSSearchField()
    /// Called with the text whenever it changes ("" when the bar closes).
    var onChange: ((String) -> Void)?

    init() {
        super.init(frame: .zero)
        material = .menu
        blendingMode = .withinWindow
        state = .active
        wantsLayer = true
        layer?.cornerRadius = 8
        layer?.borderWidth = 1
        layer?.borderColor = NSColor.separatorColor.cgColor
        isHidden = true

        field.placeholderString = "Search downloads"
        field.delegate = self
        field.sendsSearchStringImmediately = true
        field.setAccessibilityIdentifier("search")
        field.setAccessibilityLabel("Search downloads")
        field.toolTip = "Show only downloads whose name, link or note contains this text"
        field.widthAnchor.constraint(equalToConstant: 240).isActive = true
        let done = NSButton(title: "Done", target: self, action: #selector(close))
        done.bezelStyle = .rounded
        done.controlSize = .small
        done.toolTip = "Done: hide the search and show every download (Esc)"
        done.setAccessibilityIdentifier("search-done")
        let row = NSStackView(views: [field, done])
        row.spacing = 6
        row.translatesAutoresizingMaskIntoConstraints = false
        addSubview(row)
        NSLayoutConstraint.activate([
            row.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8),
            row.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            row.topAnchor.constraint(equalTo: topAnchor, constant: 6),
            row.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -6),
        ])
        setAccessibilityElement(true)
        setAccessibilityRole(.group)
        setAccessibilityLabel("Search")
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    /// Shows the bar (or keeps it) and puts the cursor in the field, its text selected.
    func open() {
        isHidden = false
        window?.makeFirstResponder(field)
        field.selectText(nil)
    }

    @objc func close() {
        isHidden = true
        field.stringValue = ""
        onChange?("")
    }

    func toggle() { isHidden ? open() : close() }

    func controlTextDidChange(_ obj: Notification) { onChange?(field.stringValue) }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        if selector == #selector(cancelOperation(_:)) {
            close()
            return true
        }
        return false
    }

    override func updateLayer() {
        super.updateLayer()
        layer?.borderColor = NSColor.separatorColor.cgColor
    }
}
