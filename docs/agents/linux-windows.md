# Building and testing the Linux and Windows apps

Part of this project's agent notes (`CLAUDE.md` points here); these rules are as binding as that file.

- Each is its own Cargo workspace (`linux/`, `windows/`), excluded from the root one, so `scripts/check.sh` never
  builds GTK or Win32; build each on its machine in a throwaway worktree of the pushed branch
  (`~/Projects/wt-hdm-linux-<topic>`, `%USERPROFILE%\Projects\wt-hdm-windows-<topic>`), then remove it. Name it
  after your task: sessions sharing one folder removed it under each other's builds (2026-11-10).
- Linux: `sh linux/scripts/build.sh [--install]` runs flatpak-builder as the Flathub app `org.flatpak.Builder` (there
  is no `flatpak-builder` command on the Linux machine); Rust comes from the `rust-stable//25.08` SDK extension. For fast
  compile loops, run cargo inside the SDK with a kept target folder: `flatpak run --filesystem=home --share=network
  --command=sh org.gnome.Sdk//49 -c "export PATH=/usr/lib/sdk/rust-stable/bin:\$PATH
  CARGO_TARGET_DIR=\$HOME/.cache/hdm-linux-target; cargo build --release"` (the host has no GTK headers). See the
  window with `--capture FILE` inside `kwin_wayland --virtual --socket <name> --exit-with-session "flatpak run
  --env=HD_DATA_DIR=… org.herbsdownloadmanager.app --capture …"` (Master Control's `exec` sets `XDG_RUNTIME_DIR`), never the desktop.
  Never change a widget inside a `borrow_mut()` of state its own signal handlers read (`set_active` on a checkbox
  runs its `toggled` handler at once): the app aborts with "RefCell already mutably borrowed". Collect the widgets,
  drop the borrow, then change them (#121, found in testing).
  App shortcuts (`set_accels_for_action`) fire before the focused text field sees the key: a plain Ctrl+A, Ctrl+C or
  Ctrl+V shortcut stole the find field's own until its actions were switched off while the field has the cursor
  (#80). Check such a key in a field with real input, not only in the list.
  Paths given to the Flatpak (`--capture`, `HD_DATA_DIR`) must be under the home folder: the sandbox has its own
  `/tmp`, so a picture "saved" to `/tmp` never appears on the host.
- Commands on the Linux and Windows machines go through Master Control: `mc-exec linux|windows "…"` (its exit code is
  the command's; long output keeps only its end), and `mc-exec MACHINE --put LOCAL REMOTE` / `--get REMOTE LOCAL`
  copy files (absolute paths: `echo %USERPROFILE%` gives the Windows home). Windows tests with several steps: write a
  `.ps1` file, copy it over and run it with `-ExecutionPolicy Bypass -File`. The shell there is `cmd`: chain commands
  with `&&`, never `;` (it became part of a file name). Inline `cmd` one-liners lose PowerShell's quotes. Start processes with `Start-Process -PassThru` and stop them by that id, never by name: Herbert's own copy
  and browser run there too. The worktree may have been removed by another session: check it exists first. Make the
  script remove an existing worktree before `git worktree add`, and print the commit it builds (`git log --oneline
  -1`): when `add` fails on a leftover folder, the build silently tests the old branch (pictures from it misled #75).
  A copy started through Master Control's `exec` runs elevated in the desktop session, and while it runs its locked
  `dist\` .exe leaves the worktree half-deleted. Stop it through Master Control, and never with
  plain `taskkill /PID n`: without `/F` that sends WM_CLOSE, which only hides Herb's DM to the tray, so the copy
  keeps running and keeps the Desktop .exe locked. Send WM_QUERYENDSESSION then WM_ENDSESSION to its window, or use
  its Quit (2026-11-18). `--capture` in session 0 (an ssh login) leaves combo boxes blank (even S7's): check their values with master-control's
  `screenshot` of the window instead (#76).
- Windows: `powershell -ExecutionPolicy Bypass -File windows\scripts\build.ps1` (copies the .exe to `dist\` and the Desktop). Scripts are blocked on the Windows machine, so every `.ps1` runs with `-ExecutionPolicy Bypass`, which lifts the block for that one run without changing the system setting. `--capture
  FILE.bmp` draws the window with WM_PRINT, which works in session 0 where PrintWindow is black; the frame
  comes out black there. Win32 constants missing from the enabled `windows` crate features can be used by value.
  `IsWindowVisible` on a child is false until its top-level window first shows: keep layout state (such as whether
  the sidebar is shown) in the app's own flags.
  Win32 controls notify synchronously (setting a ListView checkbox sends LVN_ITEMCHANGED before the call returns),
  so per-window state in a `RefCell` must be borrowed with `try_borrow_mut` in notification handlers; a second
  `borrow_mut` panics, and a `windows_subsystem` app then just vanishes with no message.
  Paths with "Herb's" in them don't survive `mc-exec windows '…'`: the apostrophe ends the shell's quoting and
  vanishes, so a command made a new "Herbs Download Manager" folder. Put such steps in a `.ps1` (`Join-Path
  $env:LOCALAPPDATA "Herb's Download Manager"`), copy it over with `mc-exec windows --put` and run that.
  A control whose state follows another's text (enabled when the link is a stream) must also be set where the form
  sets that text: the EN_CHANGE sent inside `with_form` finds the form borrowed and is dropped, so a prefilled link
  left "Only the sound" dimmed (#78).
  Don't let sibling controls overlap (a tab control's body under its pages): WM_PRINT paints siblings in its own
  order, so the pictures showed empty pages. Make the tab control just the strip and put the pages below it.
  The look lives in `theme.rs` (light and dark, Windows' app mode, the Mac's colours) and `chrome.rs` (the title row,
  toolbar, sidebar and list, all drawn). Every dialog procedure calls `theme::dialog` first; a window created visible
  calls `theme::apply` once its controls exist (its WM_SHOWWINDOW comes before them). Never pass an empty slice to
  `DrawTextW`: it reads the slice's dangling pointer and the app dies in user32 with 0xC000041D (`theme::text` skips
  empty text). Questions and messages go through `alert.rs`, never TaskDialog or MessageBox, which stay light.
  The installer: `windows\scripts\installer.ps1` after `build.ps1` (Inno Setup 6, in `~/.agents/config/tools.json`).
  To see SmartScreen, mark the file as downloaded (a `Zone.Identifier` stream with ZoneId=3) and double-click it in
  Explorer: started from a terminal, Windows shows the older "unknown publisher" warning instead.
  To update his installed copy, run the setup with `/VERYSILENT /SUPPRESSMSGBOXES /CLOSEAPPLICATIONS
  /RESTARTAPPLICATIONS` and check its exit code (5 = it couldn't close the app, and the old files stay). Started from
  master-control's `exec` the installer runs elevated and so does the copy it reopens: close that copy (WM_QUERYENDSESSION
  then WM_ENDSESSION to its window) and open it again with `explorer.exe "<path>"`, which starts it as him.
  A screenshot taken right after a menu opens catches Windows' fade-in, and a dark menu looks pale: wait a second
  before judging its colours (a "light tray menu" bug was logged by mistake that way, 2026-11-17). To open the tray
  menu without the overflow flyout, post `WM_TRAY` (WM_APP + 21) with lparam WM_RBUTTONUP to the test copy's window.

