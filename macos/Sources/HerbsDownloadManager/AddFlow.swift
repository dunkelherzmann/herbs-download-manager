import AppKit
import HDCore

/// A link about to be added, with what its server said when it was checked.
struct PendingLink {
    var url: String
    var mirrors: [String] = []
    var name: String?
    var size: UInt64?
    /// The download New Download started while it was open (`startEarly`), adopted instead of starting afresh.
    var early: UInt64?
    /// A stream a page scan found (§15): joined into one MP4 under `name`, whatever its address says.
    var stream = false
}

/// Skip, Replace or Keep Both, for a link whose file name is taken by a different file (SPECIFICATION.md §5.6).
enum ClashChoice {
    case skip, replace, keepBoth
}

extension AppModel {
    /// Adds links one at a time, so links to the same file gather into one download and its mirrors (M5). When a
    /// different file already has a link's name, in the list or in the folder, it asks Skip, Replace or Keep Both
    /// (the new one named "name - kept.ext"), with "Do the same for the rest" when more links follow, unless
    /// Settings ▸ When a file exists has the answer. The footer sums up what happened.
    func addAsking(_ links: [PendingLink], folder: URL, connections: UInt32, start: Bool, window: NSWindow?) async {
        // S7: a choice made in Settings is used without asking.
        var remembered: ClashChoice? = switch settings.whenFileExists {
        case .ask: nil
        case .keepBoth: .keepBoth
        case .skip: .skip
        case .replace: .replace
        }
        var new = 0, mirrored = 0, skipped = 0, kept = 0, replaced = 0
        // What this batch did with each file (name and size), so its other links follow: a mirror of the download
        // just added (even under a kept name), or skipped with it.
        var decided: [String: UInt64?] = [:]
        for (i, link) in links.enumerated() {
            let key = "\(link.name ?? URL(string: link.url)?.lastPathComponent ?? link.url)\u{0}\(link.size.map(String.init) ?? "")"
            if let earlier = decided[key] {
                if let early = link.early { manager.cancelEarly(token: early) }
                if let id = earlier {
                    _ = manager.addMirrors(id: id, urls: [link.url] + link.mirrors)
                    mirrored += 1
                } else {
                    skipped += 1
                }
                continue
            }
            let plan = manager.planAdd(url: link.url, name: link.name, folder: folder.path, size: link.size)
            var name = link.name
            if case .clash(let id, let path, let size, let finished, let keptName) = plan {
                let choice: ClashChoice
                if let remembered {
                    choice = remembered
                } else {
                    let asked = await ask(link: link, existingID: id, path: path, existingSize: size, finished: finished,
                                          keptName: keptName, folder: folder, more: i < links.count - 1,
                                          window: window)
                    choice = asked.choice
                    if asked.forRest { remembered = asked.choice }
                }
                switch choice {
                case .skip:
                    if let early = link.early { manager.cancelEarly(token: early) }
                    skipped += 1
                    decided[key] = .some(nil)
                    continue
                case .keepBoth:
                    name = keptName
                    kept += 1
                case .replace:
                    // The old file goes to the Trash, so replacing can be undone.
                    if let path { try? FileManager.default.trashItem(at: URL(fileURLWithPath: path), resultingItemURL: nil) }
                    if let id { _ = manager.remove(ids: [id], deleteFiles: !finished) }
                    replaced += 1
                }
            }
            let result = manager.add(url: link.url, mirrors: link.mirrors, folder: folder.path, name: name,
                                     size: link.size, connections: connections, start: start, early: link.early,
                                     stream: link.stream)
            if result.asMirror { mirrored += 1 } else { new += 1 }
            decided[key] = .some(result.id)
        }
        refresh()
        var parts: [String] = []
        if new > 0 { parts.append(new == 1 ? "1 download added" : "\(new) downloads added") }
        if mirrored > 0 { parts.append(mirrored == 1 ? "1 link added as a mirror" : "\(mirrored) links added as mirrors") }
        if kept > 0 { parts.append("\(kept) kept under a new name") }
        if replaced > 0 { parts.append("\(replaced) replaced") }
        if skipped > 0 { parts.append("\(skipped) skipped") }
        if !parts.isEmpty, links.count > 1 || mirrored > 0 || skipped > 0 { tell(parts.joined(separator: ", ")) }
    }

    private func ask(link: PendingLink, existingID: UInt64?, path: String?, existingSize: UInt64?, finished: Bool,
                     keptName: String, folder: URL, more: Bool,
                     window: NSWindow?) async -> (choice: ClashChoice, forRest: Bool) {
        let name = link.name ?? URL(string: link.url)?.lastPathComponent ?? link.url
        let alert = NSAlert()
        let place = existingID == nil ? "in \(folder.lastPathComponent)"
            : finished ? "in the list" : "in the list, not yet finished"
        alert.messageText = "A different “\(name)” is already \(place)."
        let sizes = switch (existingSize, link.size) {
        case (let a?, let b?) where a != b: " The one there is \(bytesText(bytes: a)); the new one is \(bytesText(bytes: b))."
        case (let a?, let b?) where a == b: " Both are \(bytesText(bytes: a)), but they may differ."
        default: ""
        }
        alert.informativeText = "Skip the new link, replace the existing \(finished ? "file" : "download") (it goes to "
            + "the Trash), or keep both, the new one as “\(keptName)”.\(sizes)"
        alert.addButton(withTitle: "Keep Both")
        alert.addButton(withTitle: "Skip")
        alert.addButton(withTitle: "Replace")
        alert.buttons[2].hasDestructiveAction = true
        if more {
            alert.showsSuppressionButton = true
            alert.suppressionButton?.title = "Do the same for the rest"
        }
        let response = if let window { await alert.beginSheetModal(for: window) } else { alert.runModal() }
        let choice: ClashChoice = switch response {
        case .alertSecondButtonReturn: .skip
        case .alertThirdButtonReturn: .replace
        default: .keepBoth
        }
        return (choice, alert.suppressionButton?.state == .on)
    }
}
