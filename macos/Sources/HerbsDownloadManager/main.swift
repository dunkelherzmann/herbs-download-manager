import AppKit
import HDCore

// `--self-check` prints what the core reports through the Swift bindings and exits without a window,
// so scripts/test.sh can check the Swift-to-Rust link.
if CommandLine.arguments.contains("--self-check") {
    let s = defaultSettings()
    let choices = connectionChoices()
    print("core \(coreVersion()) connections \(s.connections) per-server \(s.connectionsPerServer) "
        + "choices \(choices.first ?? 0)-\(choices.last ?? 0)")
    exit(0)
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
// The menu report test never takes focus or shows in the Dock.
app.setActivationPolicy(CommandLine.arguments.contains("--menu-report") ? .prohibited : .regular)
app.run()
