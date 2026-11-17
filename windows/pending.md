# Windows: what isn't production-ready yet

What the Windows version lacks or gets wrong compared with the macOS reference in
[`SPECIFICATION.md`](../SPECIFICATION.md), in build order, each with its spec section. Fix broken items first, one pull
request each, and tick an item with its PR number once it is built and its test passes on Windows.
[`bugs.md`](../bugs.md) and [`enhancements.md`](../enhancements.md) stay the record of requests and releases.

## Next (top priority)

- [x] **Menus in the window, matching the Mac.** The Main Menu (☰, at the toolbar's right end as on the Mac) holds every menu and submenu with the Mac's menus, order and shortcuts; no command is reachable only somewhere else. (§2, §4, enhancements.md) *Asked 2026-11-09.* Done in #39; the Edit menu and the rest matched in the parity PR.

## Parity with the Mac (checked 2026-11-17)

A side-by-side check of the Mac and Windows apps (pictures of every window, and the code item by item) found the
Windows version further from the Mac than the ticks below said. Done in the parity PR:

- [x] Light and dark, following Windows' app mode and switching while open, with the Mac window's colours: title row,
  sidebar, list, menus, scroll bars, tooltips, every dialog, check boxes. (§1, §2)
- [x] One title row as on the Mac: the title at the left, the toolbar's symbols drawn after the Mac's (link with a plus,
  filled play and pause, the queue's list, the stop circle…), Search and ☰ at its right end, the window controls after
  them (snap layouts on Maximize). (§2, §3)
- [x] The app's icon on the `.exe`, the window, the taskbar, the notification area and the alerts. (§1)
- [x] Sidebar drawn as the Mac's: chevrons, symbols, counts at the right, a rounded selection; its edge drags from 190 to
  360 pixels. (§2.1)
- [x] List drawn as the Mac's: 30-pixel rows, every other one shaded down to the bottom, the name shortened in the
  middle, a thin progress bar under unfinished names (moving while the size is unknown), Failed in red, the accent
  colour on selected rows (grey when the list hasn't the keyboard), small column titles with a sort chevron, "No
  downloads yet" / "No downloads match", the Mac's column widths, the link as a row's hover text, dates in the
  system's format, an unknown time left sorting last. (§2.2)
- [x] The Mac's Main Menu: the standard Edit menu (Undo, Redo, Cut, Copy, Paste, Select All), Hide/Show Sidebar with
  Ctrl+Alt+S, Window ▸ Bring All to Front, Hide Herb's DM. (§4)
- [x] The Mac's rules for which commands apply (the queue's buttons follow the queue; Open, Show in Folder, Rename and Move
  To need the file on disk), and its context menu: items that don't apply hidden, nothing on empty space, Open With ▸
  with the apps Windows suggests and Choose Another App…. (§3, §4.1)
- [x] Pause, Stop All and Restart ask first as on the Mac; every question and message is an alert drawn as the Mac's.
  (§4, §10)
- [x] Closing the window keeps the app running in the notification area; a second launch brings it back; Ctrl+Q quits.
  The window's size and place, the sidebar and the columns are remembered. (§2)
- [x] Settings in the Mac's order, with lists for the numbers and quarter-hour lists for the scheduler's times, no Close
  button. New Download: hints in the empty fields, Connections as a list. Batch: Continue…. Download window: a segmented
  Info | Connections | Mirrors | Options, values as plain text. (§5, §6, §9)
- [x] An installer (`windows/installer`), per user, no administrator rights; SmartScreen warns about it because it isn't
  signed, and More info ▸ Run anyway runs it. (§1)

Still different from the Mac, in build order:

- [x] "Download finished" with Open and Show in Folder buttons (a toast; now the balloon, whose click shows the file). (§9 S9) #24
- [ ] Dragging onto the window: the "Drop links here" overlay, the copy cursor only when links are found, finished
  downloads not taking mirrors, links in a dragged or copied web page's HTML. (§2, §5.4, §7.1)
- [x] Files handed to the `.exe` ("Open with", a drop on its icon) open Import Links. (§5.7) #24
- [ ] Add Links: the Folder column, a resizable window, the drop overlay. (§5.3)
- [ ] Batch: the whole help text with a separate First/Last preview, mistakes in red. (§5.5)
- [ ] Download window: resizable; right-click on a source (Copy Link, Make Main Link, Remove); the Mirrors columns
  Address and Connections; Options' Connections as a list. (§6, §7.3)
- [ ] Rename selects the name up to its extension. (§4.1)
- [ ] New Download's Save to as a folder pop-up (the folder and the folders above it). (§5.1)
- [ ] The context menu's Connections ▸ with the live connections. (§4.1)
- [ ] Hover text on the Status, Location and Note cells (the row shows its link). (§2.2)

## Bugs to fix

- [x] **A stream from a page refused with 403** (bugs.md, 2026-11-11): fixed in #110 and extension 1.18; Herbert's test passed.

## To build

The Windows version is ported once the Mac version has each feature, item by item in the Mac's build order
([`macos/pending.md`](../macos/pending.md)). Next: the five features from JDownloader 2 once the Mac has them (see its
`pending.md`), then what each item below still lists as to do.

- [x] **Project skeleton**: Rust Win32 host (`windows` crate) over the shared core, one static `.exe`, `windows\scripts\build.ps1`, copied to the desktop on every build. (§1) #36
- [x] **Download engine, segmentation, pause and resume, mirrors** (the shared core, checked on Windows: `scripts/check.sh` runs every core test there on each pull request, #27). (§7, §8, §10)
- [x] **Main window, menus, toolbar, context menu**, window controls on the right. (§2–§4) #39, made 1:1 with the Mac
  in the parity PR (above).
- [x] **Adding downloads, Download window, queue, settings, checksums.** (§5, §6, §9, §11) Checksums and S8 in #45.
  - [x] Adding downloads: New Download, Add Links, Batch, Import, clipboard, files dropped on the window (§5) #42. Links and text dragged from a browser (an OLE drop target) and links dropped on a row becoming its mirrors (row-drop PR). Still to do: .doc/.docx/.odt and .webloc files.
  - [x] Download window (segments, Info, Connections, Mirrors, Options with the speed limit), Add Mirror, Properties, Settings, finish notifications (§6, §7, §9) #44. The queue works through the toolbar and Tasks menu (§11).
- [x] **Hover text and accessibility labels on every control.** (§3, §4) #47: `windows\scripts\a11y-audit.ps1` checks every window.
- [x] **Feature tests** run on Windows. (all) #49: `powershell -File windows\scripts\features.ps1` (after `build.ps1`).

## Not checked automatically

The looks: checked from pictures of each window on Windows' screen, beside the Mac's, in dark mode (the machine's app
mode) and light mode. The installer: run from Explorer after SmartScreen's More info ▸ Run anyway.
