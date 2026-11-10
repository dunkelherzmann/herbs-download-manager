import AppKit
import HDCore
@preconcurrency import UserNotifications

/// The download list as the windows see it: the core's rows, refreshed twice a second.
@MainActor
final class AppModel {
    static let shared = AppModel()

    let manager: DownloadManager
    private(set) var rows: [DownloadRow] = []
    private var observers: [() -> Void] = []
    /// A short message for the main window's footer ("Added as a mirror of …"), shown for a few seconds.
    private(set) var notice: String?
    private var noticeTask: Task<Void, Never>?
    private var timer: Timer?
    /// Downloads that were finished at the last refresh, so a newly finished one can be announced (S9).
    private var finishedIDs: Set<UInt64>?

    /// The list lives in ~/Library/Application Support/Herb's Download Manager, or in $HD_DATA_DIR (tests use a private
    /// folder so they never touch the user's list).
    static var dataFolder: URL {
        if let dir = ProcessInfo.processInfo.environment["HD_DATA_DIR"], !dir.isEmpty {
            return URL(fileURLWithPath: dir, isDirectory: true)
        }
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        let folder = support.appendingPathComponent("Herb's Download Manager", isDirectory: true)
        // The app was called Herb's Downloader before 2026-11-09: carry its list over once.
        let old = support.appendingPathComponent("Herb's Downloader", isDirectory: true)
        let files = FileManager.default
        if !files.fileExists(atPath: folder.path), files.fileExists(atPath: old.path) {
            try? files.moveItem(at: old, to: folder)
        }
        return folder
    }

    /// Where new downloads go: S1, or when it's unset or its folder is gone (a disk not plugged in), the user's
    /// Downloads folder ($HD_DOWNLOADS_DIR in tests).
    static var downloadsFolder: URL {
        var isFolder: ObjCBool = false
        if let chosen = shared.settings.saveTo, FileManager.default.fileExists(atPath: chosen, isDirectory: &isFolder),
           isFolder.boolValue {
            return URL(fileURLWithPath: chosen, isDirectory: true)
        }
        return defaultDownloadsFolder
    }

    static var defaultDownloadsFolder: URL {
        if let dir = ProcessInfo.processInfo.environment["HD_DOWNLOADS_DIR"], !dir.isEmpty {
            return URL(fileURLWithPath: dir, isDirectory: true)
        }
        return FileManager.default.urls(for: .downloadsDirectory, in: .userDomainMask)[0]
    }

    private init() {
        do {
            manager = try DownloadManager.open(dataDir: Self.dataFolder.path)
            // A test copy never fills the user's Trash.
            if ProcessInfo.processInfo.environment["HD_DATA_DIR"] != nil {
                manager.useTrashFolder(dir: Self.dataFolder.appendingPathComponent("Trash").path)
            }
            // hdm, the command line and MCP server, reaches the app through it (§12). Without it the app still works.
            do { try manager.startControlServer(downloads: Self.defaultDownloadsFolder.path) } catch {
                NSLog("hdm can't reach the app: %@", errorText(error))
            }
            // The browser extension reaches the app through hdm (§13): tell the browsers where it is. A test copy
            // leaves them pointing at the installed app.
            if !TestCopy.isOn {
                let m = manager
                let hdm = Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/hdm").path
                DispatchQueue.global(qos: .utility).async {
                    for problem in m.registerBrowserHost(hdmPath: hdm) { NSLog("Browser extension: %@", problem) }
                }
            }
        } catch {
            let alert = NSAlert()
            alert.messageText = "Herb's Download Manager can't open its download list"
            alert.informativeText = errorText(error)
            alert.runModal()
            exit(1)
        }
        refresh()
        timer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { _ in
            MainActor.assumeIsolated { AppModel.shared.refresh() }
        }
    }

    func observe(_ callback: @escaping () -> Void) {
        observers.append(callback)
    }

    func refresh() {
        rows = manager.rows()
        let finished = Set(rows.filter { $0.status.isFinished }.map(\.id))
        if let before = finishedIDs {
            for row in rows where finished.contains(row.id) && !before.contains(row.id) { announce(row) }
        }
        finishedIDs = finished
        observers.forEach { $0() }
        askForLogins()
        QueueDone.check(manager)
    }

    /// Downloads already asked about, so a cancelled prompt isn't shown again until the download runs again.
    private var askedForLogin: Set<UInt64> = []
    private var asking = false
    /// Off while the feature tests run.
    var asksForLogins = true

    /// A download that failed on a server asking for a user name and password gets a prompt for one.
    private func askForLogins() {
        askedForLogin.formIntersection(rows.filter { $0.needsLogin != nil }.map(\.id))
        guard asksForLogins, !asking, let row = rows.first(where: { $0.needsLogin != nil && !askedForLogin.contains($0.id) }),
              let host = row.needsLogin else { return }
        askedForLogin.insert(row.id)
        asking = true
        DispatchQueue.main.async {
            MainActor.assumeIsolated {
                SignInPrompts.askForLogin(host: host)
                AppModel.shared.asking = false
            }
        }
    }

    /// The settings (§9), saved by the core in the data folder and applied as soon as they're set.
    var settings: Settings {
        get { manager.settings() }
        set { manager.setSettings(settings: newValue); refresh() }
    }

    // MARK: notifications (S9)

    /// Tests run with their own data folder and never show notifications.
    private var notificationsAllowed: Bool {
        ProcessInfo.processInfo.environment["HD_DATA_DIR"] == nil && Bundle.main.bundleIdentifier != nil
    }

    /// Asks macOS once for permission to show notifications (when S9 is turned on, or at the first finish).
    func askToNotify() {
        guard notificationsAllowed else { return }
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) { _, _ in }
    }

    private func announce(_ row: DownloadRow) {
        guard notificationsAllowed, settings.notifyWhenFinished else { return }
        let content = UNMutableNotificationContent()
        content.title = "Download finished"
        content.body = row.name
        content.sound = .default
        content.categoryIdentifier = FinishedNotifications.category
        content.userInfo = ["id": row.id]
        let id = "finished-\(row.id)"
        Task {
            let center = UNUserNotificationCenter.current()
            FinishedNotifications.shared.register()
            guard (try? await center.requestAuthorization(options: [.alert, .sound])) == true else { return }
            try? await center.add(UNNotificationRequest(identifier: id, content: content, trigger: nil))
        }
    }

    func add(url: String, start: Bool) {
        add(url: url, mirrors: [], folder: Self.downloadsFolder, name: nil,
            connections: settings.connections, start: start)
    }

    /// Adds a download; `start` runs it now (Download), otherwise it waits in the queue (Add). A link to a file
    /// that an unfinished download already fetches (same name, and same size when both are known) becomes its
    /// mirror instead, and the footer says so.
    @discardableResult
    func add(url: String, mirrors: [String], folder: URL, name: String?, size: UInt64? = nil, connections: UInt32,
             start: Bool) -> UInt64 {
        let result = manager.add(url: url, mirrors: mirrors, folder: folder.path, name: name, size: size,
                                 connections: connections, start: start, early: nil, stream: false)
        refresh()
        if result.asMirror, let row = rows.first(where: { $0.id == result.id }) {
            tell("Added as a mirror of “\(row.name)”")
        }
        return result.id
    }

    /// Adds links as mirrors of one download (Add Mirror…, or links dropped on it) and says how many were new.
    @discardableResult
    func addMirrors(_ id: UInt64, _ links: [String]) -> Int {
        let added = Int(manager.addMirrors(id: id, urls: links))
        refresh()
        let name = rows.first(where: { $0.id == id })?.name ?? "the download"
        tell(added == 0 ? "“\(name)” already uses \(links.count == 1 ? "that link" : "those links")"
                        : "Added \(added) mirror\(added == 1 ? "" : "s") to “\(name)”")
        return added
    }

    func tell(_ text: String) {
        notice = text
        observers.forEach { $0() }
        noticeTask?.cancel()
        noticeTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(6))
            guard !Task.isCancelled, let self else { return }
            self.notice = nil
            self.observers.forEach { $0() }
        }
    }

    func resume(_ ids: [UInt64]) { manager.resume(ids: ids); refresh() }
    func pause(_ ids: [UInt64]) { manager.pause(ids: ids); refresh() }
    func pauseAll() { manager.pauseAll(); refresh() }
    var queueRunning: Bool { manager.queueRunning() }
    func startQueue() { manager.startQueue(); refresh() }
    func stopQueue() { manager.stopQueue(); refresh() }
    func restart(_ ids: [UInt64]) { manager.restart(ids: ids); refresh() }
    func setConnections(_ id: UInt64, _ n: UInt32) { manager.setConnections(id: id, connections: n); refresh() }
    /// Delete (§10): with `deleteFiles`, the files go to the Trash; any that can't (a drive without one) stay where
    /// they are, and the footer says so.
    func remove(_ ids: [UInt64], deleteFiles: Bool) {
        let kept = manager.remove(ids: ids, deleteFiles: deleteFiles)
        refresh()
        if let first = kept.first {
            let name = (first as NSString).lastPathComponent
            tell(kept.count == 1 ? "“\(name)” couldn't go to the Trash, so it was left where it is"
                                 : "\(kept.count) files couldn't go to the Trash, so they were left where they are")
        }
    }
}

extension DownloadStatus {
    var isRunning: Bool {
        switch self {
        case .connecting, .downloading, .finishing: true
        default: false
        }
    }

    var canResume: Bool {
        switch self {
        case .waiting, .paused, .failed: true
        default: false
        }
    }

    var isFinished: Bool { if case .finished = self { true } else { false } }
}

/// The message of an error from the core ("404 Not Found"), without the Swift wrapping.
func errorText(_ error: Error) -> String {
    switch error as? CoreError {
    case .Io(let message)?, .Message(let message)?: message
    case nil: error.localizedDescription
    }
}
