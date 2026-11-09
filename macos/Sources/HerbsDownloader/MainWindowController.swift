import AppKit
import HDCore

/// The main window. For now it shows the empty download list's message and what the core reports;
/// the toolbar, sidebar and list (SPECIFICATION.md §2–§3) replace it.
@MainActor
final class MainWindowController: NSWindowController {
    init() {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1000, height: 620),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "Herb's Downloader"
        window.minSize = NSSize(width: 720, height: 420)
        window.setFrameAutosaveName("MainWindow")
        if window.frame.origin == .zero { window.center() }
        super.init(window: window)

        let empty = NSTextField(labelWithString: "No downloads yet")
        empty.font = .systemFont(ofSize: 17, weight: .medium)
        empty.textColor = .secondaryLabelColor

        let settings = defaultSettings()
        let footer = NSTextField(labelWithString:
            "Core \(coreVersion()) · up to \(settings.connections) connections per download")
        footer.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        footer.textColor = .tertiaryLabelColor
        footer.setAccessibilityIdentifier("footer")

        let content = NSView()
        for v in [empty, footer] {
            v.translatesAutoresizingMaskIntoConstraints = false
            content.addSubview(v)
        }
        NSLayoutConstraint.activate([
            empty.centerXAnchor.constraint(equalTo: content.centerXAnchor),
            empty.centerYAnchor.constraint(equalTo: content.centerYAnchor),
            footer.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 12),
            footer.bottomAnchor.constraint(equalTo: content.bottomAnchor, constant: -8),
        ])
        window.contentView = content
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }
}
