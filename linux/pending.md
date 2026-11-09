# Linux: what isn't production-ready yet

What the Linux version lacks or gets wrong compared with the macOS reference in
[`SPECIFICATION.md`](../SPECIFICATION.md), in build order, each with its spec section. Fix broken items first, one pull
request each, and tick an item with its PR number once it is built and its test passes on Linux.
[`bugs.md`](../bugs.md) and [`enhancements.md`](../enhancements.md) stay the record of requests and releases.

## Bugs to fix

No open bugs in `bugs.md`.

## To build

The Linux version is ported once the Mac version has each feature, item by item in the Mac's build order
([`pending.md`](../pending.md)). Everything is still to build.

- [ ] **Project skeleton**: Rust GTK 4 + libadwaita host over the shared core, Flatpak `org.herbsdownloadmanager.app`, `scripts/build-linux.sh --install`. (§1)
- [ ] **Download engine, segmentation, pause and resume, mirrors** (the shared core, checked on Linux). (§7, §8, §10)
- [ ] **Main window, menus, toolbar, context menu**, 1:1 with the Mac, window controls on the right and the main
  menu in the title bar's left corner. (§2–§4)
- [ ] **Adding downloads, Download window, queue, settings, checksums.** (§5, §6, §9, §11)
- [ ] **Hover text and accessibility labels on every control.** (§3, §4)
- [ ] **Feature tests** run on Linux. (all)

## Not checked automatically

Nothing is built yet.
