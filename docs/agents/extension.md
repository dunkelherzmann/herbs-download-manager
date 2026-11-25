# Working on the browser extension

Part of this project's agent notes (`CLAUDE.md` points here); these rules are as binding as that file.

- Its background script is tested in Node with a stand-in browser API: `node --test tests/extension`
  (`tests/extension/harness.js` loads the real `background.js` and `naming.js`; `load({tabs, fetch, native, script})`
  sets what the browser, the network, `hdm` and the page answer). `scripts/test.sh` runs it, so the check does.
  Test there, not with throwaway scripts: a new detection, ordering or naming rule gets its case in
  `background.test.js`. Tests stay out of `extension/`, which ships whole (every file in it is embedded, #83).

- The list (popup) isn't covered by the Node tests: check its look and keys with `sh tests/extension/popup-preview/serve.sh`
  (stand-in items in `stub.js`) in the app's built-in browser, light and dark, at 460 × 620. Keys pressed there stay
  in the page; never press keys on the Mac's desktop. Its emulated size shifts the page sideways in pictures: check
  `main.scrollLeft` before calling it a layout bug (2026-11-11).
- Every change to `extension/` raises `version` in `extension/manifest.json` (1.1 → 1.2): each install step copies a
  version Herbert hasn't got yet into his Downloads folder as "Herb's DM extension <version>" (his request,
  2026-11-10), and the app's Show Extension Folder copy is rewritten from the same files.
- To see the extension in a real browser without touching Herbert's: start Edge headless with a scratch profile
  (`--headless=new --user-data-dir=<scratch> --remote-debugging-port=9333 --load-extension=<ext>
  --disable-extensions-except=<ext> --disable-features=DisableLoadExtensionCommandLineSwitch`), save its PID, and
  drive the worker and pages over the DevTools protocol (Node's `WebSocket`). Its native-messaging host is still the
  user's real `hdm`, so anything that adds (a Download, the page's button) reaches his running app: check up to the
  click, and test adding with the Node harness or a stand-in page (2026-11-14).
- **YouTube never plays in a headless browser,** and YouTube accepts its token only from a playing player (stream
  protection status 2 = pending, then it stops after about 60 s). Test YouTube in a windowed test Edge started
  off-screen in the background: on the Mac `open -g -n --env HD_DATA_DIR=<test data> -a "Microsoft Edge" --args
  --user-data-dir=<scratch> --window-position=6000,6000 --disable-backgrounding-occluded-windows
  --disable-renderer-backgrounding` (plus the extension and DevTools flags above), with a `NativeMessagingHosts`
  folder inside that user-data-dir whose manifest points at the build under test. On Linux the same profile folder
  works (Edge on the desktop's `DISPLAY`, its host a script running `flatpak run --env=HD_DATA_DIR=… --command=hdm`).
  Edge on Windows reads hosts only from the registry, never the profile: there the test Edge reaches whatever
  Herbert's registration names (the Desktop `hdm.exe`, which `build.ps1` replaces), and `HD_DATA_DIR` in its
  environment steers it to the test copy; start it in the desktop session (an `/IT` scheduled task), not ssh's
  session 0. Drive the tab whose URL is `about:blank`: Edge on Windows lists a hidden built-in page first, and a video
  opened there is no tab, so the extension can't see it. Edge keeps an extension's service worker script cached by
  version: after editing the extension without raising its version, delete the test profile's `Default/Service
  Worker` folder (2026-11-15).
- **Edge's store registration (§13)** is tested the same way: on the Mac and Linux its per-user `External Extensions`
  folder lives in the user-data-dir, so a scratch profile's own folder tests it. Run the profile once first: Edge
  turns on, with no prompt, what it installs in a brand-new profile. Never test with uBlock Origin: his Mac and
  Windows Edge force-install it by policy, which looks like a pass (Dark Reader `ifoakfbpdcdoeenechcleahebpibofpc` is
  live and in no policy). On Windows Edge reads and watches `HKCU\Software\Microsoft\Edge\Extensions`, so a key
  there reaches his running Edge at once: test with an id he wouldn't mind getting, and remove the key after (2026-11-20).
- **A signed-in test with Herbert's Edge profile** needs a copy of it (Edge refuses DevTools on its default profile
  folder): copy `~/.config/microsoft-edge` whole, `Default/Service Worker` included. A copy without `ScriptCache`
  (an rsync excluding `*Cache*`) never starts the extension's worker ("Receiving end does not exist"). Point the
  copy's unpacked extension at a test folder by editing `extensions.settings.<id>.path` in its `Preferences`, and its
  host through a `NativeMessagingHosts` folder in the copy. Edge kept running a stale cached `background.js` in his
  profile after an update (bugs.md, 2026-11-16): in the worker, `typeof SCRIPT_VERSION` tells which script runs.
- Embedded content needs its own cases (bugs.md, 2026-11-16): a frame a page writes itself (`srcdoc`, `about:blank`)
  gets content scripts only with `match_about_blank`/`match_origin_as_fallback`, and the page's `document.open` there
  erases the script's window and document listeners while its globals stay (content.js adds them back); shadow roots
  hide their elements from `elementsFromPoint`. When clicking the page's button from DevTools, find where it really
  sits (`chrome.dom.openOrClosedShadowRoot(host)` in the extension's world of that frame): a letterboxed video's element
  is narrower than its player. In Edge on Windows pick the tab whose `document.visibilityState` is visible, not by
  address: once the hidden built-in page is navigated it matches too, and clicks there never land. A fresh test
  profile on Windows signs itself in to the Windows account (his favourites sync in): delete it after the test.
- After the extension restarts or is updated (`chrome.runtime.reload()` from its worker, which fires `onInstalled` with
  "update"), an open tab has two extension worlds named "Herb's Download Manager": the old copy, whose `chrome.runtime`
  is gone, and the one background.js put in. Over DevTools, read the button from the newest context id. To try an
  update without touching the installed folder, load a scratch copy with `--load-extension`, raise its version on
  disk, then reload it from the worker (2026-11-16).
- Content scripts that wait on `requestAnimationFrame` (the page's download button) never run in the app's built-in
  browser pane while it's hidden: a page test there timed out. Test them in the headless Edge test copy, moving the
  pointer with `Input.dispatchMouseEvent` (2026-11-14).
- Facebook answers a signed-out read with 400 and Instagram sends a page with no video in it, even with a browser's
  User-Agent: try page scans of such sites through the extension (which hands over the signed-in page, every frame),
  not with curl or `hdm scan` (2026-11-11). The hand-over can be several MB: `hdm`'s native-message limit and the
  control line's `MAX_LINE` must stay above the extension's `SCAN_TEXT_MAX`.
- **Right after merging a change to `extension/`, run `sh scripts/extension-everywhere.sh`**: it copies the merged
  version from origin/main into the Downloads folder of the Mac, the Linux machine and the Windows machine. Merges don't install,
  so 1.5–1.10 never reached the Mac's Downloads until he asked where they were (bugs.md, 2026-11-10).
  On the Mac it also leaves "Herb's DM extension <version> - Edge store.zip" there, the package Partner Center takes
  (a zip he made from the folder failed the store's check for `key` and `background.scripts`, 2026-11-25).
- **Publishing an update to Edge Add-ons** (only when Herbert asks): Partner Center ▸ Edge ▸ Herb's Download Manager ▸
  Update, upload the store zip (he picks the file: the built-in browser can't), Continue, then Publish. The submit
  page needs notes for certification every time: the ones in `docs/edge-add-ons.md` plus what changed. Choose "Yes,
  I need to provide … other info for testers" (the extension needs the app): "No" locks the notes field read-only,
  and typing into it then silently does nothing (1.37, 2026-11-25). Wait for
  the page to leave "Publishing" before moving on; afterwards the overview says "In review" (1.36, 2026-11-25).

