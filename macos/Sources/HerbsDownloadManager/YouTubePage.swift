import AppKit
import HDCore
import WebKit

/// A YouTube video's page, opened out of sight (SPECIFICATION.md §15) with the browser extension's YouTube script and
/// the core's bridge in it: `describe` waits until its player streams the video and returns the bridge's line about
/// it, for Add Links; `feed` downloads it the extension's way, handing each piece to the fed download that asked.
@MainActor
final class YouTubePage: NSObject, WKScriptMessageHandler, WKNavigationDelegate {
    private var view: WKWebView?
    private var window: NSWindow?
    private var timeout: DispatchWorkItem?
    private var described: CheckedContinuation<String?, Never>?
    private var wanted: WantedFeed?
    /// The pages feeding downloads, kept until each is done.
    private static var feeding: [String: YouTubePage] = [:]

    /// The bridge's line about the video the link names, or nil when its page never streamed it.
    static func describe(url: String) async -> String? {
        guard let id = youtubeVideoId(url: url) else { return nil }
        let page = YouTubePage()
        return await withCheckedContinuation { done in
            page.described = done
            page.open(youtubeWatchUrl(id: id))
        }
    }

    /// Starts feeding each fed download that has just started and needs its page (called with each refresh).
    static func feedWanted() {
        for want in feedsWanted() where feeding[want.feed] == nil {
            let page = YouTubePage()
            page.wanted = want
            feeding[want.feed] = page
            page.open(want.page)
        }
    }

    private func open(_ address: String) {
        guard let url = URL(string: address) else { return finish(failed: "The video's address isn't valid.") }
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        config.mediaTypesRequiringUserActionForPlayback = []
        HiddenPage.keepRunning(config)
        for script in youtubeScripts() {
            config.userContentController.addUserScript(
                WKUserScript(source: script, injectionTime: .atDocumentStart, forMainFrameOnly: true))
        }
        config.userContentController.add(self, name: "hdmPlayed")
        let view = WKWebView(frame: NSRect(x: 0, y: 0, width: 960, height: 540), configuration: config)
        view.navigationDelegate = self
        self.view = view
        window = HiddenPage.window(for: view)
        view.load(URLRequest(url: url))
        let item = DispatchWorkItem { [weak self] in
            self?.finish(failed: "YouTube's page didn't start the video: try again, or download it from the browser.")
        }
        timeout = item
        DispatchQueue.main.asyncAfter(deadline: .now() + .seconds(Int(youtubeWait())), execute: item)
    }

    /// Closes the page; a fed download that hasn't ended is told why (`failed`), or stays stopped (`nil`).
    private func finish(failed: String?) {
        timeout?.cancel()
        timeout = nil
        if let done = described {
            described = nil
            done.resume(returning: nil)
        }
        if let want = wanted {
            wanted = nil
            if let failed { feedEnd(feed: want.feed, failed: failed) }
            Self.feeding[want.feed] = nil
        }
        view?.stopLoading()
        view?.configuration.userContentController.removeScriptMessageHandler(forName: "hdmPlayed")
        view?.navigationDelegate = nil
        view = nil
        window?.orderOut(nil)
        window = nil
    }

    nonisolated func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
        MainActor.assumeIsolated {
            guard let line = message.body as? String,
                  let data = line.data(using: .utf8),
                  let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let kind = json["hdm"] as? String else { return }
            switch kind {
            case "video":
                if let done = described {
                    described = nil
                    done.resume(returning: line)
                    finish(failed: nil)
                } else if let want = wanted {
                    timeout?.cancel()
                    if let size = youtubeSize(line: line, soundOnly: want.soundOnly, lines: want.lines) {
                        feedExpect(feed: want.feed, size: size)
                    }
                    view?.evaluateJavaScript("__hdmYouTube.download(\(want.soundOnly), \(want.lines))")
                }
            case "piece":
                guard let want = wanted, let ack = json["ack"] as? String, let data = json["data"] as? String else { return }
                let track = (json["track"] as? NSNumber)?.uint32Value ?? 0
                let refused = feedPut(feed: want.feed, track: track, init: json["init"] as? Bool ?? false, data: data)
                let ok = refused == nil
                let quoted = String(data: (try? JSONSerialization.data(withJSONObject: [ack])) ?? Data(), encoding: .utf8) ?? "[\"\"]"
                view?.evaluateJavaScript("__hdmYouTube.ack(\(quoted)[0], \(ok))")
                // Paused, cancelled or gone: the page stops at the ack and the download needs nothing more.
                if !ok { finish(failed: nil) }
            case "done":
                guard let want = wanted else { return }
                let error = (json["error"] as? String).flatMap { $0.isEmpty ? nil : $0 }
                if error == nil, !(json["stopped"] as? Bool ?? false) { feedEnd(feed: want.feed, failed: nil) }
                finish(failed: error)
            default:
                break
            }
        }
    }

    /// The page can't open windows or download anything.
    func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
                 decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void) {
        decisionHandler(action.shouldPerformDownload ? .cancel : .allow)
    }
}

/// Where a page runs out of sight: web views outside a window, or in one the system thinks is hidden, don't play
/// their video (YouTube's then gives no full token) and slow their timers.
@MainActor
enum HiddenPage {
    static func keepRunning(_ config: WKWebViewConfiguration) {
        if #available(macOS 14.0, *) { config.preferences.inactiveSchedulingPolicy = .none }
    }

    /// A borderless window far off every screen, fully transparent and ignoring the mouse, holding `view`. It's
    /// ordered behind every window, never made key, and doesn't show in the Window menu or Mission Control.
    static func window(for view: NSView) -> NSWindow {
        let window = NSWindow(contentRect: NSRect(x: -30000, y: -30000, width: view.frame.width, height: view.frame.height),
                              styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.alphaValue = 0
        window.ignoresMouseEvents = true
        window.isExcludedFromWindowsMenu = true
        window.collectionBehavior = [.transient, .ignoresCycle, .stationary]
        window.contentView = view
        window.orderBack(nil)
        return window
    }
}
