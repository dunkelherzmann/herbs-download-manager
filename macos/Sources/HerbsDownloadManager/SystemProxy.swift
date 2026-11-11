import CFNetwork
import Foundation
import HDCore

/// §9.2: the Mac's automatic proxy setup (an automatic setup script, PAC, or Auto Proxy Discovery), asked of CFNetwork
/// for each address: it reads the system's settings, and the script is run as the system runs it. Without automatic
/// setup nothing is set here and the core's HTTP client follows the fixed proxy settings itself.
final class SystemProxy: ProxyFinder, @unchecked Sendable {
    private let settings: [String: Any]
    private let lock = NSLock()
    /// The scripts read, by address, for five minutes.
    private var scripts: [URL: (text: String, at: Date)] = [:]

    private init(settings: [String: Any]) {
        self.settings = settings
    }

    /// Sets the finder when the system (or a test copy's `--pac URL`) uses automatic setup. Call before the
    /// download manager opens, whose client is made once.
    static func install() {
        var settings = CFNetworkCopySystemProxySettings()?.takeRetainedValue() as? [String: Any] ?? [:]
        // A test copy names its own script, so the computer's real settings never change for a test.
        let args = CommandLine.arguments
        if ProcessInfo.processInfo.environment["HD_DATA_DIR"] != nil, let i = args.firstIndex(of: "--pac"), i + 1 < args.count {
            settings = [kCFNetworkProxiesProxyAutoConfigEnable as String: 1,
                        kCFNetworkProxiesProxyAutoConfigURLString as String: args[i + 1]]
        }
        let on = { (key: CFString) in (settings[key as String] as? NSNumber)?.boolValue == true }
        guard on(kCFNetworkProxiesProxyAutoConfigEnable) || on("ProxyAutoDiscoveryEnable" as CFString) else { return }
        setProxyFinder(finder: SystemProxy(settings: settings))
    }

    func proxyFor(url: String) -> String? {
        guard let address = URL(string: url) else { return nil }
        let list = CFNetworkCopyProxiesForURL(address as CFURL, settings as CFDictionary).takeRetainedValue() as? [[String: Any]] ?? []
        for proxy in list {
            if let answer = answer(proxy, for: address) { return answer }
        }
        return nil
    }

    /// One of CFNetwork's answers as a proxy URL, "direct://", or nil to try the next one.
    private func answer(_ proxy: [String: Any], for address: URL) -> String? {
        let type = proxy[kCFProxyTypeKey as String] as? String
        let host = proxy[kCFProxyHostNameKey as String] as? String
        let port = (proxy[kCFProxyPortNumberKey as String] as? NSNumber)?.intValue
        let kind = { (k: CFString) in type == k as String }
        if kind(kCFProxyTypeNone) {
            return "direct://"
        } else if kind(kCFProxyTypeHTTP) || kind(kCFProxyTypeHTTPS) {
            guard let host else { return nil }
            return "http://\(host):\(port ?? 8080)"
        } else if kind(kCFProxyTypeSOCKS) {
            guard let host else { return nil }
            return "socks5://\(host):\(port ?? 1080)"
        } else if kind(kCFProxyTypeAutoConfigurationURL) {
            guard let scriptURL = proxy[kCFProxyAutoConfigurationURLKey as String] as? URL, let text = script(scriptURL) else { return nil }
            return run(text, for: address)
        } else if kind(kCFProxyTypeAutoConfigurationJavaScript) {
            guard let text = proxy[kCFProxyAutoConfigurationJavaScriptKey as String] as? String else { return nil }
            return run(text, for: address)
        }
        return nil
    }

    private func run(_ script: String, for address: URL) -> String? {
        var error: Unmanaged<CFError>?
        guard let result = CFNetworkCopyProxiesForAutoConfigurationScript(script as CFString, address as CFURL, &error)?.takeRetainedValue(),
              let list = result as? [[String: Any]] else { return nil }
        for proxy in list where proxy[kCFProxyTypeKey as String] as? String != kCFProxyTypeAutoConfigurationURL as String {
            if let answer = answer(proxy, for: address) { return answer }
        }
        return nil
    }

    /// The script's text, read again after five minutes; nil when it can't be read (then the address goes directly).
    private func script(_ url: URL) -> String? {
        lock.lock()
        if let kept = scripts[url], Date().timeIntervalSince(kept.at) < 300 {
            lock.unlock()
            return kept.text
        }
        lock.unlock()
        guard let data = try? Data(contentsOf: url), let text = String(data: data, encoding: .utf8) else { return nil }
        lock.lock()
        scripts[url] = (text, Date())
        lock.unlock()
        return text
    }
}
