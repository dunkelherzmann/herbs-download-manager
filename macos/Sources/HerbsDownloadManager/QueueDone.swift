import AppKit
import HDCore

/// The scheduler's "When the queue is done" (SPECIFICATION.md §11.1): a minute's countdown with Cancel, then the app
/// quits, or the Mac sleeps or shuts down. A test copy never sleeps or shuts down the Mac; it only logs it.
@MainActor
enum QueueDone {
    static let seconds = 60
    private static var alert: NSAlert?
    private static var timer: Timer?
    private(set) static var pending: WhenDone?

    /// On every refresh: starts the countdown once the core says the queue is done. It's given the manager, since
    /// the first refresh runs while `AppModel.shared` is still being made.
    static func check(_ manager: DownloadManager) {
        guard pending == nil, let action = manager.takeQueueDone() else { return }
        begin(action)
    }

    private static var left = 0

    private static func button(_ action: WhenDone) -> String {
        switch action {
        case .quit: "Quit Now"
        case .sleep: "Sleep Now"
        case .shutDown: "Shut Down Now"
        case .nothing: ""
        }
    }

    private static func sentence(_ action: WhenDone, _ seconds: Int) -> String {
        switch action {
        case .quit: "Herb's Download Manager will quit in \(seconds) seconds."
        case .sleep: "Your Mac will go to sleep in \(seconds) seconds."
        case .shutDown: "Your Mac will shut down in \(seconds) seconds."
        case .nothing: ""
        }
    }

    private static func begin(_ action: WhenDone) {
        guard let main = (NSApp.delegate as? AppDelegate)?.mainWindow, let window = main.window else { return }
        pending = action
        left = seconds
        let alert = NSAlert()
        alert.messageText = "The queue is done"
        alert.informativeText = sentence(action, left)
        // Cancel is the default: pressing Return by chance never shuts the Mac down.
        alert.addButton(withTitle: "Cancel")
        alert.addButton(withTitle: button(action))
        Self.alert = alert
        main.showWindow(nil)
        if window.isMiniaturized { window.deminiaturize(nil) }
        NSApp.requestUserAttention(.informationalRequest)
        alert.beginSheetModal(for: window) { response in
            MainActor.assumeIsolated {
                timer?.invalidate()
                timer = nil
                Self.alert = nil
                pending = nil
                if response == .alertSecondButtonReturn { perform(action) }
            }
        }
        timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { _ in
            MainActor.assumeIsolated { tick() }
        }
    }

    private static func tick() {
        guard let alert, let action = pending else { return }
        left -= 1
        if left > 0 {
            alert.informativeText = sentence(action, left)
        } else {
            alert.window.sheetParent?.endSheet(alert.window, returnCode: .alertSecondButtonReturn)
        }
    }

    /// Feature tests: answers the countdown for `action` with Cancel.
    static func cancel(_ action: WhenDone) -> String? {
        guard let alert, pending == action, let parent = alert.window.sheetParent else {
            return "the app isn't counting down to \(action)"
        }
        parent.endSheet(alert.window, returnCode: .alertFirstButtonReturn)
        return nil
    }

    private static func perform(_ action: WhenDone) {
        if TestCopy.isOn, action != .quit {
            NSLog("Test copy: the queue is done; the Mac would %@ now", action == .sleep ? "sleep" : "shut down")
            return
        }
        switch action {
        case .quit: NSApp.terminate(nil)
        case .sleep: run("/usr/bin/pmset", ["sleepnow"])
        // Asks loginwindow to shut down, as the Apple menu's Shut Down does (macOS may ask once to allow it).
        case .shutDown: run("/usr/bin/osascript", ["-e", "tell application \"loginwindow\" to «event aevtrsdn»"])
        case .nothing: break
        }
    }

    private static func run(_ tool: String, _ args: [String]) {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: tool)
        p.arguments = args
        do { try p.run() } catch { NSLog("Couldn't run %@: %@", tool, error.localizedDescription) }
    }
}
