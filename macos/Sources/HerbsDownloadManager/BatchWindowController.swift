import AppKit
import HDCore

/// Batch Download (SPECIFICATION.md §5.5): a link with range placeholders, a preview of the first and last links
/// and the count, then the Add Links window.
@MainActor
final class BatchWindowController: NSWindowController, NSTextFieldDelegate {
    private let pattern = NSTextField()
    private let preview = NSTextField(wrappingLabelWithString: "")
    private let next = NSButton(title: "Continue…", target: nil, action: nil)
    private var links: [String] = []
    private let onLinks: ([String]) -> Void
    private var onClose: (() -> Void)?

    init(onLinks: @escaping ([String]) -> Void, onClose: @escaping () -> Void) {
        self.onLinks = onLinks
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 600, height: 230),
                              styleMask: [.titled, .closable], backing: .buffered, defer: false)
        window.title = "Batch Download"
        window.setAccessibilityIdentifier("batch-download")
        super.init(window: window)
        self.onClose = onClose
        build()
        window.center()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    private func build() {
        guard let window else { return }
        let help = NSTextField(wrappingLabelWithString:
            "Put a range where the number or letters change: ^(###1-100) pads to 3 digits, ^(0-ffH) counts in "
            + "hexadecimal, ^(a-z) or ^(AA-BZ) in letters. Separate several ranges with commas.")
        help.textColor = .secondaryLabelColor
        help.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        pattern.placeholderString = "https://example.com/img^(###1-100).jpg"
        pattern.delegate = self
        pattern.setAccessibilityIdentifier("batch-pattern")
        pattern.toolTip = "Batch link: a link with a range placeholder"
        preview.textColor = .secondaryLabelColor
        preview.setAccessibilityIdentifier("batch-preview")
        preview.font = .monospacedSystemFont(ofSize: NSFont.smallSystemFontSize, weight: .regular)

        let cancel = NSButton(title: "Cancel", target: self, action: #selector(cancel(_:)))
        cancel.toolTip = "Close without adding anything (Esc)"
        cancel.keyEquivalent = "\u{1b}"
        next.target = self
        next.action = #selector(continueToLinks(_:))
        next.keyEquivalent = "\r"
        next.isEnabled = false
        next.toolTip = "Show the links in the Add Links window"
        let buttons = NSStackView(views: [NSView(), cancel, next])

        let stack = NSStackView(views: [pattern, help, preview, buttons])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 12
        stack.edgeInsets = NSEdgeInsets(top: 20, left: 20, bottom: 20, right: 20)
        stack.translatesAutoresizingMaskIntoConstraints = false
        let content = NSView()
        content.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            stack.topAnchor.constraint(equalTo: content.topAnchor),
            stack.bottomAnchor.constraint(equalTo: content.bottomAnchor),
            pattern.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -40),
            help.widthAnchor.constraint(equalTo: pattern.widthAnchor),
            preview.widthAnchor.constraint(equalTo: pattern.widthAnchor),
            buttons.widthAnchor.constraint(equalTo: pattern.widthAnchor),
        ])
        window.contentView = content
        window.initialFirstResponder = pattern
        // As tall as the fields need, no taller.
        content.layoutSubtreeIfNeeded()
        window.setContentSize(NSSize(width: window.frame.width, height: content.fittingSize.height))
    }

    func controlTextDidChange(_ notification: Notification) {
        let text = pattern.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { preview.stringValue = ""; next.isEnabled = false; return }
        do {
            links = try expandBatch(pattern: text)
            let first = links.first ?? "", last = links.last ?? ""
            preview.stringValue = links.count == 1 ? "1 link: \(first)"
                : "\(links.count) links\nFirst: \(first)\nLast:  \(last)"
            preview.textColor = .secondaryLabelColor
            next.isEnabled = true
        } catch {
            links = []
            preview.stringValue = errorText(error)
            preview.textColor = .systemRed
            next.isEnabled = false
        }
    }

    @objc func cancel(_ sender: Any?) { close() }

    @objc func continueToLinks(_ sender: Any?) {
        let found = links
        close()
        onLinks(found)
    }

    override func close() {
        super.close()
        onClose?()
        onClose = nil
    }
}
