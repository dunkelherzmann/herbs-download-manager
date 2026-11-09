import AppKit

/// Keep on Top (§3 T11, View ▸ Keep on Top ⌃⌘T): while on, every window of the app floats above other apps'
/// windows, full-screen ones included, whether or not the app is active. Remembered between launches.
@MainActor
enum KeepOnTop {
    private static let key = "KeepOnTop"
    private static var observer: Any?
    /// A test copy (run with $HD_DATA_DIR) keeps its own setting, so it never changes Herbert's.
    private static let defaults: UserDefaults = ProcessInfo.processInfo.environment["HD_DATA_DIR"] == nil
        ? .standard : UserDefaults(suiteName: "com.herbertcisneros.HerbsDownloadManager.tests") ?? .standard

    static var isOn: Bool {
        get { defaults.bool(forKey: key) }
        set {
            defaults.set(newValue, forKey: key)
            NSApp.windows.forEach(apply)
        }
    }

    /// Applies the setting to every window now and to each window as it appears (dialogs, the Download window).
    static func install() {
        NSApp.windows.forEach(apply)
        guard observer == nil else { return }
        observer = NotificationCenter.default.addObserver(forName: NSWindow.didUpdateNotification, object: nil,
                                                          queue: .main) { note in
            guard let window = note.object as? NSWindow else { return }
            MainActor.assumeIsolated { apply(window) }
        }
    }

    private static func apply(_ window: NSWindow) {
        // Sheets and menus keep their own level.
        guard window.sheetParent == nil, window.level == .normal || window.level == .floating else { return }
        let level: NSWindow.Level = isOn ? .floating : .normal
        if window.level != level { window.level = level }
        if isOn {
            window.collectionBehavior.insert(.fullScreenAuxiliary)
        } else {
            window.collectionBehavior.remove(.fullScreenAuxiliary)
        }
    }
}
