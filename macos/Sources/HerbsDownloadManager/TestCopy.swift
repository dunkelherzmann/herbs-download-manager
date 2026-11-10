import Foundation

/// A test copy is the app run with $HD_DATA_DIR: it keeps its own list and its own window and column layout, so it
/// never changes Herbert's.
enum TestCopy {
    static let isOn = ProcessInfo.processInfo.environment["HD_DATA_DIR"] != nil

    /// The name a window, split view or table saves its layout under (in the app's standard defaults).
    static func autosave(_ name: String) -> String {
        isOn ? "\(name) (test copy)" : name
    }
}
