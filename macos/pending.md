# Mac: what isn't production-ready yet

The macOS version is the reference: its features are the ones in [`SPECIFICATION.md`](../SPECIFICATION.md). This file lists
what the Mac version still lacks or gets wrong against that list and the project's rules, in build order, each with
its spec section. Fix broken items first, one pull request each, and tick an item with its PR number once it is built
and its test passes on the Mac. [`bugs.md`](../bugs.md) and [`enhancements.md`](../enhancements.md) stay the record of requests and
releases.

## Next (top priority)

- [x] **Every menu in the window, none in the macOS menu bar.** (#19) The menu bar (File, Tasks, Tools, Help and their submenus) moves into the left of the window's title bar, as on Linux and Windows, so nothing needs a trip to the top of a wide screen. The macOS menu bar keeps only the app menu macOS always shows. Every shortcut keeps working, Undo, Cut, Copy, Paste and Select All in text fields included, and every item keeps its hover text and VoiceOver label. Describe the new menu in `SPECIFICATION.md` in the same PR. (§2, §4, enhancements.md) *Asked 2026-11-09.*
- [x] **One Main Menu (☰) button instead of the menu bar in the title bar,** as in Herb's Terminal, at the right end of the toolbar. (#26) (§2, §3 T9, §4, enhancements.md) *Asked 2026-11-09.*

## Bugs to fix

No open bugs in `bugs.md`.

## To build

- [x] **Project skeleton**: Rust core (Cargo workspace) with UniFFI Swift bindings, Swift 6 AppKit app,
  `macos/scripts/build.sh`, `scripts/test.sh`, the `hd-mock` test server (ranges on/off, mirrors, changing files,
  stalls, connection limits). (§1) #4
- [x] **Single-source download engine**: one connection, `.hdpart` file, rename at the end. (§8) #5
- [x] **Dynamic segmentation**: up to N connections, largest-range halving, 256 KiB minimum, takeover of silent
  connections. (§8) #5
- [x] **Pause and resume in the engine**: resume state (link, file, size, validator, ranges) saved on pause, every
  5 s and on failure, after flushing the data to disk; a new run continues from it, fetching only the remaining
  ranges; starts over when the `.hdpart` file is gone or the server can't resume. (§10) #6
- [x] **Mirrors in the engine**: sources per download, mirror check (size, ranges, per-source validator), spreading
  connections by host, retries and failover, refused hosts, slow connections handing their ranges to a faster
  source, turning sources on and off, a mirror rescuing a link without ranges. (§7) #7
- [x] **Add mirrors before, during and after in the engine** (M1–M3), including an expired link finished by a
  mirror added while paused. (§7.1, §10) #7
- [x] **Mirrors in the app**: the Mirrors field, Add Mirror… (⌥⌘M), the Mirrors tab with its sources list. (§5.1,
  §7.1, §7.3) #14
- [x] **Main window**: title bar, toolbar, sidebar filters, download list with columns, footer, search. (§2, §3) #8
  New Download, the queue buttons and Settings stay disabled until their items below.
- [x] **Menus and context menu** with shortcuts. (§4) #10
  Connections ▸ Add/Remove Connection change a running download at once; Restart discards and starts again.
  Items for windows not built yet (New Download, Add Mirror, Properties, Settings, queue) stay disabled.
- [x] **Pause and resume in the app**: running downloads come back Paused at launch, the can't-resume question, the
  quit question, Restart. (§10) #11
- [x] **New Download, Add Links, clipboard, drag and drop, Batch Download.** (§5) #13
  The Mirrors field and "Treat as mirrors of one file" (M1) are in; Add Mirror… and the Mirrors tab stay with
  "Mirrors in the app".
- [x] **Download window**: segments bar, Info, Connections, Mirrors and Options tabs. (§6) #14
  The Options tab has the connections picker; the per-download speed limit and checksum come with **Settings**
  and **Checksum verification** below.
- [x] **Queue**: Start Queue, Stop Queue, downloads at once. (§11) — PR #16
- [x] **Settings**: the Settings window (⌘,), `settings.json`, speed limits for all downloads and for one. (§9, §6) — PR #23
- [x] **Checksum verification** and checksum-file lookup. (§6, §9 S8) #45
- [x] **Hover text, accessibility labels and menu commands on every control.** (§3, §4) #47: `scripts/test.sh` runs an audit of every window (`--a11y-report`).
- [x] **App icon** in the Mac, Linux and Windows formats. (§1) — PR #18 (the Linux and Windows apps use it when they are built)
- [x] **Feature tests** driving the app's own commands against the mock server, results per feature. (all) #49: `tests/features.scenario`, run by `macos/scripts/features.sh` (also in `scripts/test.sh`).
- [x] **Free space check** before a download starts, and the FAT32 4 GB warning. (enhancements.md)
- [x] **Checksums the server announces** (`Digest`, `Repr-Digest`, `Content-MD5`, `x-goog-hash`). (§6, enhancements.md)
- [x] **Remembered connection limits per server.** (§7.4, §8, enhancements.md)
- [x] **Overlap check on resume** for servers without validators. (§10, enhancements.md)
- [x] **Links that need a login**: per-site user name and password in the app's own encrypted store, cookies, referer. (enhancements.md)
- [x] **Finished downloads**: Open and Show in Finder on the notification, Delete Completed, Move/Rename, server's file date. (enhancements.md)
- [x] **Start while New Download is open.** (§5.1, enhancements.md)
- [x] **A note per download**, in a column and the search. (§2.2, §4.1, enhancements.md; #55)
- [x] **Command line and MCP server.** (enhancements.md, SPECIFICATION.md §12; #57)
- [x] **Scheduler** for the queue. (§11, enhancements.md)
- [ ] **Browser extension** (Safari, Chrome, Firefox). (enhancements.md)

## Not checked automatically

The main window opening: checked by hand from a capture of the window.
