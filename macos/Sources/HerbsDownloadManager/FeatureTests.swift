import AppKit
import HDCore

/// Feature tests (tests/features.scenario, run by core/src/scenario.rs): every step runs on the main thread through the
/// app's own code, the code its buttons, menus and dialogs run. `--features SCRIPT OUT NAME=VALUE…` runs the scenario,
/// writes the report to OUT and quits.
final class FeatureHost: ScenarioHost, @unchecked Sendable {
    func command(name: String, ids: [UInt64], args: [String]) -> String? {
        let done = DispatchSemaphore(value: 0)
        nonisolated(unsafe) var problem: String?
        Task { @MainActor in
            problem = await Self.run(name, ids, args)
            done.signal()
        }
        done.wait()
        return problem
    }

    @MainActor
    private static func run(_ name: String, _ ids: [UInt64], _ args: [String]) async -> String? {
        let model = AppModel.shared
        guard let window = (NSApp.delegate as? AppDelegate)?.mainWindow else { return "no main window" }
        // The list's rows are refreshed twice a second: read them now, so a command sees each row's current state.
        model.refresh()
        window.list.select(ids)
        switch name {
        case "add", "add-as-mirrors":
            // New Download and Add Links end in addAsking; Add Links' "Treat as mirrors" makes one link with mirrors.
            guard args.count >= 3 else { return "add needs a folder, start or queue, and links" }
            let folder = URL(fileURLWithPath: args[0], isDirectory: true)
            let links = Array(args[2...])
            let pending = name == "add"
                ? links.map { PendingLink(url: $0) }
                : [PendingLink(url: links[0], mirrors: Array(links.dropFirst()))]
            await model.addAsking(pending, folder: folder, connections: model.settings.connections,
                                  start: args[1] == "start", window: nil)
        case "resume": window.resumeSelected(nil)
        case "pause": window.pauseSelected(nil)
        case "restart": model.restart(ids)  // Restart's question answered Restart
        case "delete": model.remove(ids, deleteFiles: false)  // Delete's question answered Remove from List
        case "delete-files": model.remove(ids, deleteFiles: true)  // … answered Move to Trash Too
        case "start-queue": window.startQueue(nil)
        case "stop-queue": window.stopQueue(nil)
        case "stop-all": window.stopAll(nil)
        case "add-mirror": if let id = ids.first { model.addMirrors(id, args) }  // Add Mirror's sheet, Add
        case "set-checksum":
            guard let id = ids.first, args.count == 2 else { return "set-checksum needs an algorithm and a value" }
            let algorithm: ChecksumAlgorithm? = switch args[0].lowercased() {
            case "md5": .md5
            case "sha1", "sha-1": .sha1
            case "sha256", "sha-256": .sha256
            case "sha512", "sha-512": .sha512
            default: nil
            }
            return model.manager.setChecksum(id: id, algorithm: algorithm, value: args[1])  // the Options tab's field
        default: return "no command \(name)"
        }
        return nil
    }

    /// `--features SCRIPT OUT NAME=VALUE…`
    @MainActor
    static func runIfAsked() {
        let args = CommandLine.arguments
        guard let i = args.firstIndex(of: "--features"), i + 2 < args.count,
              let script = try? String(contentsOfFile: args[i + 1], encoding: .utf8) else { return }
        let out = args[i + 2]
        let vars = args[(i + 3)...].compactMap { pair -> ScenarioVar? in
            guard let eq = pair.firstIndex(of: "=") else { return nil }
            return ScenarioVar(name: String(pair[..<eq]), value: String(pair[pair.index(after: eq)...]))
        }
        let manager = AppModel.shared.manager
        // The scenario gives sign-ins itself; a prompt would stop the app's thread.
        AppModel.shared.asksForLogins = false
        Thread.detachNewThread {
            let report = manager.runScenario(host: FeatureHost(), script: script, vars: vars)
            try? report.write(toFile: out, atomically: true, encoding: .utf8)
            exit(report.contains(", 0 failed") ? 0 : 1)
        }
    }
}
