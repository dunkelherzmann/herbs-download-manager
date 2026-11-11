# Windows: what isn't production-ready yet

What the Windows version lacks or gets wrong compared with the macOS reference in
[`SPECIFICATION.md`](../SPECIFICATION.md), in build order, each with its spec section. Fix broken items first, one pull
request each, and tick an item with its PR number once it is built and its test passes on Windows.
[`bugs.md`](../bugs.md) and [`enhancements.md`](../enhancements.md) stay the record of requests and releases.

## Next (top priority)

- [x] **Menus in the window, matching the Mac.** The Main Menu (☰, in the top-left corner since the Mac moved to one, #26) holds every menu and submenu with the Mac's menus, order and shortcuts; no command is reachable only somewhere else. The skeleton has the menus' titles; their items come with the main window. (§2, §4, enhancements.md) *Asked 2026-11-09.* Done in #39.

## Bugs to fix

- [x] **A stream from a page refused with 403** (bugs.md, 2026-11-11): fixed in #110 and extension 1.18; Herbert's test passed.

## To build

The Windows version is ported once the Mac version has each feature, item by item in the Mac's build order
([`macos/pending.md`](../macos/pending.md)). Next: the five features from JDownloader 2 once the Mac has them (see its
`pending.md`), then what each item below still lists as to do.

- [x] **Project skeleton**: Rust Win32 host (`windows` crate) over the shared core, one static portable `.exe`, `windows\scripts\build.ps1`, copied to the desktop on every build. (§1) #36
- [x] **Download engine, segmentation, pause and resume, mirrors** (the shared core, checked on Windows: `scripts/check.sh` runs every core test there on each pull request, #27). (§7, §8, §10)
- [x] **Main window, menus, toolbar, context menu**, 1:1 with the Mac, window controls on the right and the main
  menu in the title bar's left corner. (§2–§4) #39. The progress bar under a name and dropping links come later (the list shows the percentage in Status meanwhile).
- [x] **Adding downloads, Download window, queue, settings, checksums.** (§5, §6, §9, §11) Checksums and S8 in #45.
  - [x] Adding downloads: New Download, Add Links, Batch, Import, clipboard, files dropped on the window (§5) #42. Links and text dragged from a browser (an OLE drop target) and links dropped on a row becoming its mirrors (row-drop PR). Still to do: .doc/.docx/.odt and .webloc files.
  - [x] Download window (segments, Info, Connections, Mirrors, Options with the speed limit), Add Mirror, Properties, Settings, finish notifications (§6, §7, §9) #44. The queue works through the toolbar and Tasks menu (§11).
- [x] **Hover text and accessibility labels on every control.** (§3, §4) #47: `windows\scripts\a11y-audit.ps1` checks every window.
- [x] **Feature tests** run on Windows. (all) #49: `powershell -File windows\scripts\features.ps1` (after `build.ps1`).

## Not checked automatically

The main window: checked from its `--capture` picture (WM_PRINT over SSH).
