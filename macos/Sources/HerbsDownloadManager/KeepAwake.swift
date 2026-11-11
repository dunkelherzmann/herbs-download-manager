import Foundation

/// The Mac doesn't go to sleep while a download runs (SPECIFICATION.md §10): an activity that keeps the system from
/// idle sleep is held while any download is active, and ended when none is. The display still sleeps as set.
@MainActor
enum KeepAwake {
    private static var activity: NSObjectProtocol?

    static func set(_ downloading: Bool) {
        if downloading, activity == nil {
            activity = ProcessInfo.processInfo.beginActivity(options: .idleSystemSleepDisabled, reason: "Downloads are running")
        } else if !downloading, let a = activity {
            ProcessInfo.processInfo.endActivity(a)
            activity = nil
        }
    }
}
