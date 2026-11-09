import AppKit
import HDCore

/// Links that need a login (enhancements.md): asking for a site's user name and password when a download meets a
/// 401, the Administrator Password that encrypts saved sign-ins (as in Herb's Terminal), and the Sign-ins window.
@MainActor
enum SignInPrompts {
    static var manager: DownloadManager { AppModel.shared.manager }

    static func secureField(_ placeholder: String) -> NSSecureTextField {
        let f = NSSecureTextField(frame: NSRect(x: 0, y: 0, width: 280, height: 24))
        f.placeholderString = placeholder
        f.toolTip = placeholder
        return f
    }

    static func textField(_ placeholder: String, _ value: String = "", help: String? = nil) -> NSTextField {
        let f = NSTextField(frame: NSRect(x: 0, y: 0, width: 280, height: 24))
        f.placeholderString = placeholder
        f.stringValue = value
        f.toolTip = help ?? placeholder
        return f
    }

    static func stack(_ views: [NSView]) -> NSStackView {
        let s = NSStackView(views: views)
        s.orientation = .vertical
        s.alignment = .leading
        s.spacing = 8
        for v in views where v is NSTextField { v.widthAnchor.constraint(equalToConstant: 280).isActive = true }
        s.frame = NSRect(x: 0, y: 0, width: 280, height: CGFloat(views.count) * 32)
        return s
    }

    /// Runs `work` off the main thread (deriving the key takes about a second) while the app waits.
    static func slow<T: Sendable>(_ work: @escaping @Sendable () -> T) -> T {
        let done = DispatchSemaphore(value: 0)
        nonisolated(unsafe) var result: T?
        DispatchQueue.global().async { result = work(); done.signal() }
        while done.wait(timeout: .now()) == .timedOut {
            RunLoop.current.run(mode: .modalPanel, before: Date(timeIntervalSinceNow: 0.05))
        }
        return result!
    }

    /// The first-use explanation. Returns true once a protection has been chosen.
    @discardableResult
    static func explainSetup() -> Bool {
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = "Create an Administrator Password"
        alert.informativeText = """
        Herb's Download Manager keeps saved sign-ins inside the app. To encrypt them, create an Administrator \
        Password; you'll enter it once each time you open the app, when a download first needs a saved sign-in.

        Without one, saved sign-ins (passwords and cookies included) are stored as plain text, which is a security \
        risk: anyone or anything that can read your files could read them.
        """
        alert.addButton(withTitle: "Create Administrator Password…")
        alert.addButton(withTitle: "Store as Plain Text")
        alert.addButton(withTitle: "Not Now").keyEquivalent = "\u{1B}"
        switch alert.runModal() {
        case .alertFirstButtonReturn:
            return createOrModify()
        case .alertSecondButtonReturn:
            // The risky choice is never one click away.
            let confirm = NSAlert()
            confirm.alertStyle = .critical
            confirm.messageText = "Store saved sign-ins as plain text?"
            confirm.informativeText = "Passwords and cookies you save will be readable by anyone or anything that can read your files. You can create an Administrator Password later in Settings ▸ Sign-ins."
            confirm.addButton(withTitle: "Cancel")
            confirm.addButton(withTitle: "Store as Plain Text")
            guard confirm.runModal() == .alertSecondButtonReturn else { return false }
            do { try manager.storeSignInsAsPlainText(); return true } catch { show(error); return false }
        default:
            return false
        }
    }

    /// Creates the Administrator Password, or changes it after checking the current one.
    @discardableResult
    static func createOrModify() -> Bool {
        let store = manager.signInStore()
        let modifying = store == .locked || store == .unlocked
        var message: String?
        while true {
            let alert = NSAlert()
            alert.messageText = modifying ? "Change Administrator Password" : "Create Administrator Password"
            alert.informativeText = (message.map { $0 + "\n\n" } ?? "")
                + (modifying ? "Saved sign-ins are encrypted again with the new password.\n\nLeave both new password fields blank to remove the Administrator Password: saved sign-ins are then stored as plain text, unprotected."
                             : "Saved sign-ins will be encrypted with this password. It can't be recovered if you forget it; you would have to delete your sign-ins and enter them again.")
            let current = secureField("Current password")
            let new = secureField(modifying ? "New password (blank to remove)" : "New password (at least 8 characters)")
            let confirm = secureField("Confirm new password")
            alert.accessoryView = stack(modifying ? [current, new, confirm] : [new, confirm])
            alert.window.initialFirstResponder = modifying ? current : new
            alert.addButton(withTitle: modifying ? "Change Password" : "Create Password")
            alert.addButton(withTitle: "Cancel")
            guard alert.runModal() == .alertFirstButtonReturn else { return false }
            let (old, newValue) = (current.stringValue, new.stringValue)
            guard newValue == confirm.stringValue else { message = "The new passwords don't match."; continue }
            let m = manager
            if modifying {
                guard slow({ m.unlockSignIns(password: old) }) else { message = "The current password is wrong."; continue }
                if newValue.isEmpty, !confirmRemoval() { continue }
            }
            do {
                if modifying {
                    try slow { Result { try m.changeAdminPassword(password: newValue) } }.get()
                } else {
                    try slow { Result { try m.createAdminPassword(password: newValue) } }.get()
                }
                return true
            } catch {
                message = errorText(error)
            }
        }
    }

    private static func confirmRemoval() -> Bool {
        let alert = NSAlert()
        alert.alertStyle = .critical
        alert.messageText = "Remove the Administrator Password?"
        alert.informativeText = "Saved sign-ins, passwords and cookies included, will be stored as plain text: anyone or anything that can read your files could read them."
        // Cancel is the default, so Return never removes the protection by accident.
        alert.addButton(withTitle: "Cancel")
        alert.addButton(withTitle: "Remove Password").hasDestructiveAction = true
        return alert.runModal() == .alertSecondButtonReturn
    }

    /// Asks for the Administrator Password to unlock saved sign-ins for this session.
    @discardableResult
    static func unlock(detail: String? = nil) -> Bool {
        var message: String?
        while manager.signInStore() == .locked {
            let alert = NSAlert()
            alert.messageText = "Unlock Saved Sign-ins"
            alert.informativeText = message ?? detail ?? "Enter the Administrator Password to use your saved sign-ins."
            let field = secureField("Administrator Password")
            alert.accessoryView = stack([field])
            alert.window.initialFirstResponder = field
            alert.addButton(withTitle: "Unlock")
            alert.addButton(withTitle: "Cancel")
            guard alert.runModal() == .alertFirstButtonReturn else { return false }
            let (m, password) = (manager, field.stringValue)
            if !slow({ m.unlockSignIns(password: password) }) { message = "That isn't the Administrator Password." }
        }
        return true
    }

    /// Before saving: a protection is chosen, and encrypted sign-ins are unlocked.
    static func readyToSave() -> Bool {
        switch manager.signInStore() {
        case .notChosen: return explainSetup()
        case .locked: return unlock()
        case .plain, .unlocked: return true
        }
    }

    /// A download met a server that asks for a user name and password. With a saved sign-in that's locked, asks for
    /// the Administrator Password; otherwise for the site's user name and password. Then starts it again.
    static func askForLogin(host: String) {
        let site = siteOf(text: host)
        let saved = manager.signIns().first { $0.site == site || host.hasSuffix("." + $0.site) }
        if let saved, saved.password == nil {
            if unlock(detail: "A download from \(host) needs your saved sign-in. Enter the Administrator Password to use it.") {
                manager.resumeWaitingForLogin(site: saved.site)
            }
            return
        }
        let alert = NSAlert()
        alert.messageText = "\(host) asks for a user name and password"
        alert.informativeText = saved == nil ? "Sign in to download the file." : "The saved sign-in for \(saved!.site) wasn't accepted. Sign in again."
        let user = textField("User name", saved?.user ?? "")
        let password = secureField("Password")
        let remember = NSButton(checkboxWithTitle: "Remember for \(site)", target: nil, action: nil)
        remember.state = .on
        remember.toolTip = "Save this sign-in in the app (encrypted with the Administrator Password) for later downloads from \(site)"
        alert.accessoryView = stack([user, password, remember])
        alert.window.initialFirstResponder = saved == nil ? user : password
        alert.addButton(withTitle: "Sign In")
        alert.addButton(withTitle: "Cancel")
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        let sign = NewSignIn(site: saved?.site ?? site, user: user.stringValue, password: password.stringValue,
                             cookies: saved?.cookies ?? "", referer: saved?.referer ?? "")
        if remember.state == .on, readyToSave() {
            do { try manager.saveSignIn(signIn: sign) } catch { show(error); manager.useSignInForSession(signIn: sign) }
        } else {
            manager.useSignInForSession(signIn: sign)
        }
        manager.resumeWaitingForLogin(site: sign.site)
    }

    static func show(_ error: Error) {
        let alert = NSAlert()
        alert.messageText = errorText(error)
        alert.runModal()
    }
}

/// Settings ▸ Sign-ins ▸ Manage…: every saved sign-in, with Add, Edit, Delete, Lock or Unlock, and the Administrator
/// Password.
@MainActor
final class SignInsWindowController: NSWindowController, NSTableViewDataSource, NSTableViewDelegate {
    static let shared = SignInsWindowController()

    private let table = NSTableView()
    private var rows: [SignInRow] = []
    private let status = NSTextField(labelWithString: "")
    private let add = NSButton(title: "Add…", target: nil, action: nil)
    private let edit = NSButton(title: "Edit…", target: nil, action: nil)
    private let delete = NSButton(title: "Delete", target: nil, action: nil)
    private let lock = NSButton(title: "Unlock…", target: nil, action: nil)
    private let password = NSButton(title: "Administrator Password…", target: nil, action: nil)

    private init() {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 620, height: 340),
                              styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        window.title = "Sign-ins"
        window.minSize = NSSize(width: 520, height: 260)
        super.init(window: window)
        build()
    }

    required init?(coder: NSCoder) { fatalError() }

    func show() {
        reload()
        window?.center()
        showWindow(nil)
        window?.makeKeyAndOrderFront(nil)
    }

    private func build() {
        guard let content = window?.contentView else { return }
        for (id, title, width) in [("site", "Site", 170.0), ("user", "User name", 140.0), ("password", "Password", 90.0),
                                   ("extra", "Cookies and referer", 110.0), ("stored", "Stored", 90.0)] {
            let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier(id))
            column.title = title
            column.width = width
            table.addTableColumn(column)
        }
        table.dataSource = self
        table.delegate = self
        table.usesAlternatingRowBackgroundColors = true
        table.doubleAction = #selector(editSignIn(_:))
        table.target = self
        table.setAccessibilityLabel("Saved sign-ins")
        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.borderType = .bezelBorder
        let buttons: [(NSButton, Selector, String)] = [
            (add, #selector(addSignIn(_:)), "Add a site's user name, password, cookies or referer"),
            (edit, #selector(editSignIn(_:)), "Change the selected sign-in"),
            (delete, #selector(deleteSignIn(_:)), "Delete the selected sign-in from the app"),
            (lock, #selector(toggleLock(_:)), "Unlock saved sign-ins with the Administrator Password, or lock them again"),
            (password, #selector(changePassword(_:)), "Create, change or remove the Administrator Password that encrypts saved sign-ins"),
        ]
        for (b, action, help) in buttons {
            b.target = self
            b.action = action
            b.toolTip = help
            b.bezelStyle = .rounded
        }
        let bar = NSStackView(views: [add, edit, delete, NSView(), lock, password])
        bar.spacing = 8
        status.textColor = .secondaryLabelColor
        status.lineBreakMode = .byWordWrapping
        let all = NSStackView(views: [bar, scroll, status])
        all.orientation = .vertical
        all.alignment = .leading
        all.spacing = 10
        all.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(all)
        NSLayoutConstraint.activate([
            all.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 16),
            all.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -16),
            all.topAnchor.constraint(equalTo: content.topAnchor, constant: 14),
            all.bottomAnchor.constraint(equalTo: content.bottomAnchor, constant: -14),
            bar.widthAnchor.constraint(equalTo: all.widthAnchor),
            scroll.widthAnchor.constraint(equalTo: all.widthAnchor),
            status.widthAnchor.constraint(equalTo: all.widthAnchor),
        ])
    }

    func reload() {
        let m = AppModel.shared.manager
        rows = m.signIns()
        table.reloadData()
        let store = m.signInStore()
        lock.title = store == .unlocked ? "Lock" : "Unlock…"
        lock.isEnabled = store == .locked || store == .unlocked
        password.title = store == .locked || store == .unlocked ? "Change Administrator Password…" : "Create Administrator Password…"
        status.stringValue = switch store {
        case .notChosen: "Sign-ins are asked for when a download needs one. Create an Administrator Password to encrypt the ones you save."
        case .plain: "Saved sign-ins are stored as plain text. Create an Administrator Password to encrypt them."
        case .locked: "Saved sign-ins are encrypted and locked. Unlock them to see or change passwords."
        case .unlocked: "Saved sign-ins are encrypted with the Administrator Password and unlocked until the app quits."
        }
        selectionChanged()
        SettingsWindowController.shared.showSignIns()
    }

    private func selectionChanged() {
        let one = table.selectedRow >= 0
        edit.isEnabled = one && rows[table.selectedRow].password != nil
        delete.isEnabled = one
    }

    func tableViewSelectionDidChange(_ notification: Notification) { selectionChanged() }

    func numberOfRows(in tableView: NSTableView) -> Int { rows.count }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let r = rows[row]
        let text: String = switch tableColumn?.identifier.rawValue {
        case "site": r.site
        case "user": r.user
        case "password": r.password == nil ? "Locked" : (r.password!.isEmpty ? "" : "••••••••")
        case "extra": [r.cookies.map { $0.isEmpty ? nil : "Cookies" } ?? (r.encrypted ? nil : nil),
                       r.referer.isEmpty ? nil : "Referer"].compactMap { $0 }.joined(separator: ", ")
        default: r.encrypted ? "Encrypted" : "Plain text"
        }
        let cell = NSTextField(labelWithString: text)
        cell.lineBreakMode = .byTruncatingTail
        return cell
    }

    @objc private func addSignIn(_ sender: Any?) { form(nil) }

    @objc private func editSignIn(_ sender: Any?) {
        guard table.selectedRow >= 0 else { return }
        let r = rows[table.selectedRow]
        if r.password == nil { guard SignInPrompts.unlock() else { return }; reload() }
        form(rows.first { $0.site == r.site })
    }

    private func form(_ existing: SignInRow?) {
        guard SignInPrompts.readyToSave() else { reload(); return }
        var message: String?
        while true {
            let alert = NSAlert()
            alert.messageText = existing == nil ? "Add a Sign-in" : "Edit the Sign-in for \(existing!.site)"
            alert.informativeText = message ?? "Downloads from the site and its subdomains send these. Leave out what the site doesn't need."
            let site = SignInPrompts.textField("Site, such as example.com", existing?.site ?? "")
            let user = SignInPrompts.textField("User name", existing?.user ?? "")
            let password = SignInPrompts.secureField("Password")
            password.stringValue = existing?.password ?? ""
            let cookies = SignInPrompts.textField("Cookies, such as session=abc123", existing?.cookies ?? "",
                                                  help: "The Cookie header downloads from this site send, as name=value pairs separated by semicolons")
            let referer = SignInPrompts.textField("Referer, the page links come from", existing?.referer ?? "",
                                                  help: "For sites that only allow downloads coming from one of their pages")
            alert.accessoryView = SignInPrompts.stack([site, user, password, cookies, referer])
            alert.window.initialFirstResponder = existing == nil ? site : user
            alert.addButton(withTitle: "Save")
            alert.addButton(withTitle: "Cancel")
            guard alert.runModal() == .alertFirstButtonReturn else { return }
            let new = NewSignIn(site: site.stringValue, user: user.stringValue, password: password.stringValue,
                                cookies: cookies.stringValue, referer: referer.stringValue)
            do {
                if let existing, siteOf(text: site.stringValue) != existing.site { try AppModel.shared.manager.deleteSignIn(site: existing.site) }
                try AppModel.shared.manager.saveSignIn(signIn: new)
                AppModel.shared.manager.resumeWaitingForLogin(site: siteOf(text: site.stringValue))
                reload()
                return
            } catch {
                message = errorText(error)
            }
        }
    }

    @objc private func deleteSignIn(_ sender: Any?) {
        guard table.selectedRow >= 0 else { return }
        let r = rows[table.selectedRow]
        let alert = NSAlert()
        alert.messageText = "Delete the sign-in for \(r.site)?"
        alert.informativeText = "Its user name, password, cookies and referer are removed from the app. This can't be undone."
        alert.addButton(withTitle: "Cancel")
        alert.addButton(withTitle: "Delete").hasDestructiveAction = true
        guard alert.runModal() == .alertSecondButtonReturn else { return }
        do { try AppModel.shared.manager.deleteSignIn(site: r.site) } catch { SignInPrompts.show(error) }
        reload()
    }

    @objc private func toggleLock(_ sender: Any?) {
        if AppModel.shared.manager.signInStore() == .unlocked {
            AppModel.shared.manager.lockSignIns()
        } else if SignInPrompts.unlock() {
            AppModel.shared.manager.resumeWaitingForLogin(site: "")
        }
        reload()
    }

    @objc private func changePassword(_ sender: Any?) {
        let store = AppModel.shared.manager.signInStore()
        if store == .notChosen { SignInPrompts.explainSetup() } else { SignInPrompts.createOrModify() }
        reload()
    }
}
