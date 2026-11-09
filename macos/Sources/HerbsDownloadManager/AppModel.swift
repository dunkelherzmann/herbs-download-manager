import AppKit
import HDCore

/// The download list as the windows see it: the core's rows, refreshed twice a second.
@MainActor
final class AppModel {
    static let shared = AppModel()

    let manager: DownloadManager
    private(set) var rows: [DownloadRow] = []
    private var observers: [() -> Void] = []
    private var timer: Timer?

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

    /// S1 when unset: the user's Downloads folder ($HD_DOWNLOADS_DIR in tests).
    static var downloadsFolder: URL {
        if let dir = ProcessInfo.processInfo.environment["HD_DOWNLOADS_DIR"], !dir.isEmpty {
            return URL(fileURLWithPath: dir, isDirectory: true)
        }
        return FileManager.default.urls(for: .downloadsDirectory, in: .userDomainMask)[0]
    }

    private init() {
        do {
            manager = try DownloadManager.open(dataDir: Self.dataFolder.path)
        } catch {
            let alert = NSAlert()
            alert.messageText = "Herb's Download Manager can't open its download list"
            alert.informativeText = "\(error)"
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
        observers.forEach { $0() }
    }

    func add(url: String, start: Bool) {
        _ = manager.add(url: url, mirrors: [], folder: Self.downloadsFolder.path, name: nil,
                        connections: defaultSettings().connections, start: start)
        refresh()
    }

    func resume(_ ids: [UInt64]) { manager.resume(ids: ids); refresh() }
    func pause(_ ids: [UInt64]) { manager.pause(ids: ids); refresh() }
    func pauseAll() { manager.pauseAll(); refresh() }
    func remove(_ ids: [UInt64], deleteFiles: Bool) { manager.remove(ids: ids, deleteFiles: deleteFiles); refresh() }
}

extension DownloadStatus {
    var isRunning: Bool {
        switch self {
        case .connecting, .downloading: true
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
