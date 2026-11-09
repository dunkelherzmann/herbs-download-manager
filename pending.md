# Mac: what isn't production-ready yet

The macOS version is the reference: its features are the ones in [`SPECIFICATION.md`](SPECIFICATION.md). This file lists
what the Mac version still lacks or gets wrong against that list and the project's rules, in build order, each with
its spec section. Fix broken items first, one pull request each, and tick an item with its PR number once it is built
and its test passes on the Mac. [`bugs.md`](bugs.md) and [`enhancements.md`](enhancements.md) stay the record of requests and
releases.

## Bugs to fix

No open bugs in `bugs.md`.

## To build

- [x] **Project skeleton**: Rust core (Cargo workspace) with UniFFI Swift bindings, Swift 6 AppKit app,
  `scripts/build-macos.sh`, `scripts/test.sh`, the `hd-mock` test server (ranges on/off, mirrors, changing files,
  stalls, connection limits). (§1) #4
- [ ] **Single-source download engine**: one connection, `.hdpart` file, rename at the end. (§8)
- [ ] **Dynamic segmentation**: up to N connections, largest-range halving, 256 KiB minimum, takeover of silent
  connections. (§8)
- [ ] **Pause and resume**: state saved on pause, every 5 s and on quit; resume after relaunch; can't-resume
  question. (§10)
- [ ] **Mirrors**: sources per download, mirror check (size, ranges, per-source validator), spreading connections,
  retries and failover, sources list. (§7)
- [ ] **Add mirrors before, during and after** (M1–M3). (§7.1)
- [ ] **Main window**: title bar, toolbar, sidebar filters, download list with columns, footer, search. (§2, §3)
- [ ] **Menus and context menu** with shortcuts. (§4)
- [ ] **New Download, Add Links, clipboard, drag and drop, Batch Download.** (§5)
- [ ] **Download window**: segments bar, Info, Connections, Mirrors and Options tabs. (§6)
- [ ] **Queue**: Start Queue, Stop Queue, downloads at once. (§11)
- [ ] **Settings**. (§9)
- [ ] **Checksum verification** and checksum-file lookup. (§6, §9 S8)
- [ ] **Hover text, accessibility labels and menu commands on every control.** (§3, §4)
- [ ] **App icon** in the Mac, Linux and Windows formats. (§1)
- [ ] **Feature tests** driving the app's own commands against the mock server, results per feature. (all)

## Not checked automatically

The main window opening: checked by hand from a capture of the window.
