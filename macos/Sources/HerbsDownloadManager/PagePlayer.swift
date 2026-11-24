import Foundation
import HDCore
import WebKit

/// A page whose player builds its video's address only while it runs (SPECIFICATION.md §15): opened out of sight,
/// muted, in a web view of its own that keeps nothing (no cookies or cache shared with anything), for a few seconds,
/// noting what its frames fetch or play. What it saw goes back to the core's page scan.
@MainActor
final class PagePlayer: NSObject, WKScriptMessageHandler, WKNavigationDelegate {
    private var view: WKWebView?
    private var seen: [String] = []
    private var done: CheckedContinuation<[String], Never>?
    private var stop: DispatchWorkItem?

    /// What the page at `url` fetched or played (one line of the catch script each), after `playedWait()`.
    static func run(url: String) async -> [String] {
        guard let address = URL(string: url) else { return [] }
        let player = PagePlayer()
        return await withCheckedContinuation { player.start(address, done: $0) }
    }

    private func start(_ url: URL, done: CheckedContinuation<[String], Never>) {
        self.done = done
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        config.mediaTypesRequiringUserActionForPlayback = []
        config.userContentController.addUserScript(
            WKUserScript(source: playedCatchScript(), injectionTime: .atDocumentStart, forMainFrameOnly: false))
        config.userContentController.add(self, name: "hdmPlayed")
        let view = WKWebView(frame: NSRect(x: 0, y: 0, width: 960, height: 540), configuration: config)
        view.navigationDelegate = self
        self.view = view
        view.load(URLRequest(url: url))
        finish(after: .seconds(Int(playedWait()[0])))
    }

    /// Ends the run after `delay`, replacing any end set before.
    private func finish(after delay: DispatchTimeInterval) {
        stop?.cancel()
        let item = DispatchWorkItem { [weak self] in self?.end() }
        stop = item
        DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: item)
    }

    private func end() {
        guard let done else { return }
        self.done = nil
        stop?.cancel()
        view?.stopLoading()
        view?.configuration.userContentController.removeScriptMessageHandler(forName: "hdmPlayed")
        view?.navigationDelegate = nil
        view = nil
        done.resume(returning: seen)
    }

    nonisolated func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
        guard let line = message.body as? String else { return }
        MainActor.assumeIsolated {
            guard done != nil, !seen.contains(line) else { return }
            let first = !seen.contains { playedIsPlaylist(seen: $0) }
            seen.append(line)
            // Once a playlist turned up, its other qualities and sound follow within moments.
            if first, playedIsPlaylist(seen: line) { finish(after: .milliseconds(Int(playedWait()[1]))) }
        }
    }

    /// The page can't open windows or download anything.
    func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
                 decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void) {
        decisionHandler(action.shouldPerformDownload ? .cancel : .allow)
    }

    func webView(_ webView: WKWebView, decidePolicyFor response: WKNavigationResponse,
                 decisionHandler: @escaping @MainActor (WKNavigationResponsePolicy) -> Void) {
        decisionHandler(response.canShowMIMEType ? .allow : .cancel)
    }
}
