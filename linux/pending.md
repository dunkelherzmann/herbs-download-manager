# Linux: what isn't production-ready yet

What the Linux version lacks or gets wrong compared with the macOS reference in
[`SPECIFICATION.md`](../SPECIFICATION.md), in build order, each with its spec section. Fix broken items first, one pull
request each, and tick an item with its PR number once it is built and its test passes on Linux.
[`bugs.md`](../bugs.md) and [`enhancements.md`](../enhancements.md) stay the record of requests and releases.

## Next (top priority)

- [x] **Menus in the window, matching the Mac.** The Main Menu (☰, in the title bar's left corner since the Mac moved to one, #26) holds every menu and submenu with the Mac's menus, order and shortcuts; no command is reachable only somewhere else. The skeleton has the menus' titles; their items come with the main window. (§2, §4, enhancements.md) *Asked 2026-11-09.* Done in #38.

## Bugs to fix

No open bugs in `bugs.md`.

## To build

The Linux version is ported once the Mac version has each feature, item by item in the Mac's build order
([`macos/pending.md`](../macos/pending.md)). Everything is still to build.

- [x] **Project skeleton**: Rust GTK 4 + libadwaita host over the shared core, Flatpak `org.herbsdownloadmanager.app`, `linux/scripts/build.sh --install`. (§1) #35
- [x] **Download engine, segmentation, pause and resume, mirrors** (the shared core, checked on Linux: `scripts/check.sh` runs every core test there on each pull request, #27). (§7, §8, §10)
- [x] **Main window, menus, toolbar, context menu**, 1:1 with the Mac, window controls on the right and the main
  menu in the title bar's left corner. (§2–§4) #38. Edit has Copy Link, Select All and Find (text fields keep GTK's own Cut/Copy/Paste); Keep on Top stays disabled (GTK 4/Wayland); dropping links comes with Adding downloads.
- [ ] **Adding downloads, Download window, queue, settings, checksums.** (§5, §6, §9, §11)
  - [x] Adding downloads: New Download, Add Links, Batch, Import, clipboard, drop on the window (§5) #41. Still to do: reading .doc/.docx/.odt and .webloc files, and links dropped on a row becoming its mirrors.
  - [x] Download window (segments, Info, Connections, Mirrors, Options with the speed limit), Add Mirror, Properties, Settings, finish notifications (§6, §7, §9) #43. The queue works through the toolbar and Tasks menu (§11). Still to do: checksums (§6, S8).
- [ ] **Hover text and accessibility labels on every control.** (§3, §4)
- [ ] **Feature tests** run on Linux. (all)

## Not checked automatically

The main window: checked from its `--capture` picture taken in a virtual KWin session.
