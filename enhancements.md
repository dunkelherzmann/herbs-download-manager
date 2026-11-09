# Enhancements

Every enhancement requested for Herb's Downloader, oldest first. New requests are added at the end and nothing is removed.
A box is ticked once the change has shipped. A request that covers several platforms has a box for each, and its main box is ticked when they're all done.

- [x] **Set up the project, locally and on GitHub,** with `SPECIFICATION.md`, `bugs.md`, `enhancements.md` and `pending.md`. *Requested 2026-11-08.* Done in #1.
- [x] **Study Internet Download Manager, Free Download Manager, XDM, Progressive Downloader and AB Download Manager** with Craft Anything and REA, Progressive Downloader as the main inspiration, and record their features in the spec. *Requested 2026-11-08.* Done in #1 (`SPECIFICATION.md` §12).
- [ ] **Multi-threaded downloading:** many connections per file, split dynamically. *Requested 2026-11-08.* Mac, Linux and Windows. (§8)
  - [ ] Mac
  - [ ] Linux
  - [ ] Windows
- [ ] **Mirror support:** one file from several sources, with connections spread over them. *Requested 2026-11-08.* Mac, Linux and Windows. (§7)
  - [ ] Mac
  - [ ] Linux
  - [ ] Windows
- [ ] **Add mirrors before, during and after adding a download.** *Requested 2026-11-08.* Mac, Linux and Windows. (§7.1)
  - [ ] Mac
  - [ ] Linux
  - [ ] Windows
- [ ] **Stop and resume downloads.** *Requested 2026-11-08.* Mac, Linux and Windows. (§10)
  - [ ] Mac
  - [ ] Linux
  - [ ] Windows
- [x] **No tags.** *Requested 2026-11-08.* The spec has fixed status filters and file-type groups only (§2.1). Done in #1.
- [ ] **Menu bar, toolbar, icons and UI based on AB Download Manager.** *Requested 2026-11-08.* Mac, Linux and Windows. (§2–§4)
  - [ ] Mac
  - [ ] Linux
  - [ ] Windows
- [x] **Study IDM on the Windows PC with Craft Anything and REA** instead of only its website: its menus, toolbar, dialogs and strings from `IDMan.exe`, and its main window while running. *Requested 2026-11-08.* Done in #2 (`SPECIFICATION.md` §12).
- [x] **Up to 32 connections per download,** chosen from 1 to 32. *Requested 2026-11-09.* Spec: 32 by default for downloads and per server, with a 1–32 picker (§5.1 A5, §9 S2–S3). Done in #3.
- [ ] **Use the most appropriate, portable language, evaluating Rust.** *Requested 2026-11-09.* Spec: Rust core on all three platforms, Swift/AppKit host on the Mac, Rust Win32 and GTK 4 hosts (§1). Done in #3; built per platform below.
  - [ ] Mac
  - [ ] Linux
  - [ ] Windows
- [ ] **Build the Mac app first for testing,** before Linux and Windows. *Requested 2026-11-09.* In progress, in the order of `pending.md`: the skeleton is done in #4.
