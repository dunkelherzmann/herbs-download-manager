# Linux: what isn't production-ready yet

What the Linux version lacks or gets wrong compared with the macOS reference in
[`SPECIFICATION.md`](../SPECIFICATION.md), in build order, each with its spec section. Fix broken items first, one pull
request each, and tick an item with its PR number once it is built and its test passes on Linux.
[`bugs.md`](../bugs.md) and [`enhancements.md`](../enhancements.md) stay the record of requests and releases.

## Next (top priority)

- [x] **Menus in the window, matching the Mac.** The Main Menu (☰, in the title bar's left corner since the Mac moved to one, #26) holds every menu and submenu with the Mac's menus, order and shortcuts; no command is reachable only somewhere else. The skeleton has the menus' titles; their items come with the main window. (§2, §4, enhancements.md) *Asked 2026-11-09.* Done in #38.

## Parity with the Mac (checked 2026-11-24)

A code-by-code check of the Linux app against the Mac's found these differences. Done in the parity PR:

- [x] Closing the window leaves the app running (downloads carry on); opening it again brings the window back; Quit and
  Ctrl+Q ask first while downloads run. The window's size, the sidebar and the columns' widths, order and sort are
  remembered. (§2)
- [x] The list sorts by Date Added at first (it sorted by Mirrors); Failed in red; hover text on Status and Location;
  Location as `~/…`; links dropped on a finished download open Add Links. The sidebar's file types have symbols and
  counts. "Drop links here" while links are dragged over the window. (§2, §2.1, §2.2, §5.4)
- [x] The Mac's rules for which commands apply: the queue's buttons follow the queue, Stop All the queue too, Restart
  any selection, Open, Show in Folder, Rename and Move To a file on disk. Search opens and closes the find bar; the
  search field has hover text. (§3)
- [x] The standard Edit menu (Undo, Redo, Cut, Copy, Paste, Select All, Find) acting on the text field with the cursor;
  Ctrl+C in the list no longer copies links (Copy Link stays in the context menu, as on the Mac); while a text field
  (also in Note or Rename) has the cursor, Delete and F2 stay the field's. View says Hide Sidebar or Show Sidebar.
  About has the Mac's lines and the core's version. (§4)
- [x] The context menu as the Mac builds it: right-clicking a download selects it, nothing on empty space, Open With ▸
  (Choose an App… in the Flatpak), Rename and Move To only for one finished download, Connections ▸ with the live
  lines and hidden for finished ones, Restart… and Delete…. (§4.1)
- [x] Pause, Stop All and the Download window's Pause ask before pausing a download that can't be resumed; Restart asks
  before discarding. (§10)
- [x] Add from Clipboard and Import Links with no links, and an Export error, are alerts as on the Mac (Export says
  nothing when it works); the clipboard's HTML counts too. New Download, Add Links and Batch close with Esc; New
  Download's hints; Add Links' cells and summary have hover text. Batch: the help text, First/Last, mistakes in red,
  Continue…. (§5)
- [x] Download window: column titles over Connections and Mirrors, right-click on a source (Copy Link, Make Main Link,
  Remove), Remove and Make Main Link only for a selected mirror, cells' hover text, failures in red, the segments bar's
  and Add Mirror's hover text, Options off for a finished download. (§6, §7.3)
- [x] Settings: hover text on every number, the scheduler's times and days, the folder (its whole path) and the speed;
  Show Extension Folder's error is an alert. Removing the Administrator Password asks first. (§9, §9.1)
- [x] `linux/scripts/a11y-audit.sh` counts GTK's "button" role, so it reads the whole main window and the Download
  window.

Still different from the Mac:

- [x] Add Links has no Folder column. (§5.3) Done in #55.
- [x] Files handed to the app (Open With from the file manager) don't open Import Links: the `.desktop` file takes no
  files and the app doesn't handle them. (§5.7) Done in #55.

## Bugs to fix

- [x] **A running download's count stands still while a speed limit holds it back** (bugs.md, 2026-11-24): the downloaded amount only moves when the download is paused (shared core). Fixed in #51.
- [x] **A stream from a page refused with 403** (bugs.md, 2026-11-11): fixed in #110 and extension 1.18; Herbert's test passed.
- [x] **Feature tests failed once under load** (bugs.md, found in testing 2026-11-10): 4 of 17 failed while the machine was busy; probably a timing assumption in the scenario.

## To build

The Linux version is ported once the Mac version has each feature, item by item in the Mac's build order
([`macos/pending.md`](../macos/pending.md)). Next: the five features from JDownloader 2 once the Mac has them (see its
`pending.md`), then what each item below still lists as to do.

- [x] **Project skeleton**: Rust GTK 4 + libadwaita host over the shared core, Flatpak `org.herbsdownloadmanager.app`, `linux/scripts/build.sh --install`. (§1) #35
- [x] **Download engine, segmentation, pause and resume, mirrors** (the shared core, checked on Linux: `scripts/check.sh` runs every core test there on each pull request, #27). (§7, §8, §10)
- [x] **Main window, menus, toolbar, context menu**, 1:1 with the Mac, window controls on the right and the main
  menu in the title bar's left corner. (§2–§4) #38. Keep on Top stays disabled (GTK 4/Wayland).
- [x] **Adding downloads, Download window, queue, settings, checksums.** (§5, §6, §9, §11) Checksums and S8 in #45.
  - [x] Adding downloads: New Download, Add Links, Batch, Import, clipboard, drop on the window (§5) #41. Links dropped on a row become its mirrors (row-drop PR). Import and drops read .doc, .docx, .odt, .rtfd, web archives and .webloc files as on the Mac #41.
  - [x] Download window (segments, Info, Connections, Mirrors, Options with the speed limit), Add Mirror, Properties, Settings, finish notifications (§6, §7, §9) #43. The queue works through the toolbar and Tasks menu (§11).
- [x] **Hover text and accessibility labels on every control.** (§3, §4) #47: `linux/scripts/a11y-audit.sh` reads every window through AT-SPI.
- [x] **Feature tests** run on Linux. (all) #49: `sh linux/scripts/features.sh` (after `build.sh --install`).

## Not checked automatically

The main window: checked from its `--capture` picture taken in a virtual KWin session.
