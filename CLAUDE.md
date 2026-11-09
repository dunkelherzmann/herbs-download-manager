# Notes for agents working on Herb's Download Manager

Read `~/.agents/library/herberts-preferences/SKILL.md` first (Herbert's global CLAUDE.md says so). `SPECIFICATION.md`
is the product, `macos/pending.md` the Mac build order, `enhancements.md` and `bugs.md` the record of requests.

## Checking and merging

- No GitHub CI (the repo is private and Actions minutes cost money): check a pushed branch with `scripts/check.sh`,
  which runs the Mac build and tests here and `cargo test --workspace` on linux and windows, then merge.
- Never run `cargo fmt`: there is no `rustfmt.toml` and the code isn't rustfmt-formatted (about 120-column lines);
  it rewrites the whole workspace.

## Trying the app

- Never quit, relaunch or click in Herbert's running copy. Run a test copy from `dist/` with `HD_DATA_DIR` (and
  `HD_DOWNLOADS_DIR`) set to a scratch folder: it keeps its own list and its own UI settings. Read windows by the
  test copy's pid (`CGWindowListCopyWindowInfo`), or with computer-use when his copy isn't running. `build.sh
  --install` leaves his running copy alone; the new build loads at his next launch.

## Building the Linux and Windows apps

- Each is its own Cargo workspace (`linux/`, `windows/`), excluded from the root one, so `scripts/check.sh` never
  builds GTK or Win32; build each on its machine in a throwaway worktree of the pushed branch
  (`~/Projects/wt-hdm-linux`, `%USERPROFILE%\Projects\wt-hdm-windows`), then remove it.
- Linux: `sh linux/scripts/build.sh [--install]` runs flatpak-builder as the Flathub app `org.flatpak.Builder` (there
  is no `flatpak-builder` command on linux); Rust comes from the `rust-stable//25.08` SDK extension. See the
  window with `--capture FILE` inside `kwin_wayland --virtual --socket <name> --exit-with-session "flatpak run
  --env=HD_DATA_DIR=… org.herbsdownloadmanager.app --capture …"` (set `XDG_RUNTIME_DIR` over ssh), never the desktop.
- Windows: `powershell -File windows\scripts\build.ps1` (copies the .exe to `dist\` and the Desktop). `--capture
  FILE.bmp` draws the window with WM_PRINT, which works over ssh (session 0) where PrintWindow is black; the frame
  comes out black there. Win32 constants missing from the enabled `windows` crate features can be used by value.

## Writing tests (learned the hard way)

- `hd-mock` servers that fake a dying or changed file must also cut responses already under way; otherwise "source
  dies midway" tests pass or fail by timing and never exercise recovery (#7).
- A test that acts once a download reaches N bytes must pick N well before the event it races with: progress counts
  only bytes written in `write_buffer` pieces, so the counter can reach N just as the event fires.
- Don't bound the bytes a server sent tightly when checking that nothing is fetched twice: data already in flight to
  a connection that was cut short counts as sent. Compare against a full refetch instead.
- Anything measured per tick (speeds, slow connections) must count bytes received, not written (#25).
- A test that fails now and then: run it 30–60 times on the machine where it fails to get the real error first
  (Windows "access denied" on a name being deleted, #30).
