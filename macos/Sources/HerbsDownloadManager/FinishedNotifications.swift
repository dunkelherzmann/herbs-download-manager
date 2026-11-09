import AppKit
import HDCore
@preconcurrency import UserNotifications

/// The "Download finished" notification's buttons (enhancements.md): Open and Show in Finder. Clicking the
/// notification itself shows the file in Finder too.
final class FinishedNotifications: NSObject, UNUserNotificationCenterDelegate, @unchecked Sendable {
    static let shared = FinishedNotifications()
    static let category = "finished"
    private var registered = false

    func register() {
        guard !registered else { return }
        registered = true
        let center = UNUserNotificationCenter.current()
        let open = UNNotificationAction(identifier: "open", title: "Open", options: [.foreground])
        let show = UNNotificationAction(identifier: "show", title: "Show in Finder", options: [.foreground])
        center.setNotificationCategories([UNNotificationCategory(identifier: Self.category, actions: [open, show],
                                                                 intentIdentifiers: [])])
        center.delegate = self
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
                                withCompletionHandler done: @escaping () -> Void) {
        let id = (response.notification.request.content.userInfo["id"] as? NSNumber)?.uint64Value
        let action = response.actionIdentifier
        done()
        DispatchQueue.main.async {
            MainActor.assumeIsolated {
                // The file may have been renamed or moved since: look it up now.
                guard let id, let path = AppModel.shared.rows.first(where: { $0.id == id })?.path,
                      FileManager.default.fileExists(atPath: path) else { return }
                let url = URL(fileURLWithPath: path)
                if action == "open" {
                    NSWorkspace.shared.open(url)
                } else if action != UNNotificationDismissActionIdentifier {
                    NSWorkspace.shared.activateFileViewerSelecting([url])
                }
            }
        }
    }

    /// Shown while the app is in front too.
    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification,
                                withCompletionHandler done: @escaping (UNNotificationPresentationOptions) -> Void) {
        done([.banner, .sound])
    }
}
