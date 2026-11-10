# Notes for agents working on Herb's Download Manager

Read `~/.agents/library/herberts-preferences/SKILL.md` first (Herbert's global CLAUDE.md says so). `SPECIFICATION.md`
is the product, `macos/pending.md` the Mac build order, `enhancements.md` and `bugs.md` the record of requests.

## Checking and merging

- The repository is public (since 2026-11-10). Releases: `scripts/release.sh` builds and checks the four files from
  origin/main (HDM_RELEASE_REF=origin/BRANCH to try a branch), and `scripts/release.sh --publish` tags v<version> and
  publishes the pre-release. On Linux, `flatpak-builder --run` exits 1 even for `true`: judge a check run there by
  its output.

- GitHub Actions checks every pull request again (free since the repo went public, 2026-11-10;
  `.github/workflows/build.yml`). Before merging, also check a pushed branch with `scripts/check.sh`, which runs the Mac build and tests here and `cargo test --workspace` on linux and windows, then merge.
  Merge only on its exit status 0 and an `ok` line for every machine: never judge it through `| tail`, which
  returns tail's status (#89 merged with Linux and Windows failing, 2026-11-10).
  After `gh pr merge`, confirm the PR says MERGED before removing its worktree or branch: a merge refused for a
  conflict, then the branch deleted, closed #100 unmerged and the installs built main without it (2026-11-10).
- Other sessions often work in this same folder: do your own work in a worktree of your own from the start (`git
  worktree add ~/Projects/wt-hdm-<topic> -b <branch> origin/main`), and run `scripts/check.sh` there. In the shared
  checkout, another session switched the branch under a running task: a commit landed on local main and the check
  tested the other session's commit (#64).
- Never run `cargo fmt`: there is no `rustfmt.toml` and the code isn't rustfmt-formatted (about 120-column lines);
  it rewrites the whole workspace.

## Trying the app

- Never quit, relaunch or click in Herbert's running copy. Run a test copy from `dist/` with `HD_DATA_DIR` (and
  `HD_DOWNLOADS_DIR`) set to a scratch folder: it keeps its own list and its own UI settings. Read windows by the
  test copy's pid (`CGWindowListCopyWindowInfo`), or with computer-use when his copy isn't running. `build.sh
  --install` leaves his running copy alone; the new build loads at his next launch.
- Code reached while `AppModel` is being made (its first `refresh()`, observers, anything `init` calls) must not read
  `AppModel.shared`: the app traps at launch with "dispatch_once ... trying to lock recursively". Pass the model or its
  manager in instead (hit twice: hdm's control server, the scheduler's countdown).
- AppKit saves window frames, split views and table columns (`setFrameAutosaveName`, `autosaveName`) in the app's
  standard defaults, which test copies share with his copy: name them through `TestCopy.autosave(_:)` so a test run
  never changes his layout. A new list column needs a one-time move into place, since a saved layout puts it last.

## Building the Linux and Windows apps

- Each is its own Cargo workspace (`linux/`, `windows/`), excluded from the root one, so `scripts/check.sh` never
  builds GTK or Win32; build each on its machine in a throwaway worktree of the pushed branch
  (`~/Projects/wt-hdm-linux-<topic>`, `%USERPROFILE%\Projects\wt-hdm-windows-<topic>`), then remove it. Name it
  after your task: sessions sharing one folder removed it under each other's builds (2026-11-10).
- Linux: `sh linux/scripts/build.sh [--install]` runs flatpak-builder as the Flathub app `org.flatpak.Builder` (there
  is no `flatpak-builder` command on linux); Rust comes from the `rust-stable//25.08` SDK extension. For fast
  compile loops, run cargo inside the SDK with a kept target folder: `flatpak run --filesystem=home --share=network
  --command=sh org.gnome.Sdk//49 -c "export PATH=/usr/lib/sdk/rust-stable/bin:\$PATH
  CARGO_TARGET_DIR=\$HOME/.cache/hdm-linux-target; cargo build --release"` (the host has no GTK headers). See the
  window with `--capture FILE` inside `kwin_wayland --virtual --socket <name> --exit-with-session "flatpak run
  --env=HD_DATA_DIR=… org.herbsdownloadmanager.app --capture …"` (set `XDG_RUNTIME_DIR` over ssh), never the desktop.
  App shortcuts (`set_accels_for_action`) fire before the focused text field sees the key: a plain Ctrl+A, Ctrl+C or
  Ctrl+V shortcut stole the find field's own until its actions were switched off while the field has the cursor
  (#80). Check such a key in a field with real input, not only in the list.
  Paths given to the Flatpak (`--capture`, `HD_DATA_DIR`) must be under the home folder: the sandbox has its own
  `/tmp`, so a picture "saved" to `/tmp` never appears on the host.
- Windows tests over ssh with several steps: write a `.ps1` file, copy it over and run it with `-ExecutionPolicy Bypass
  -File`. The ssh shell there is `cmd`: chain commands with `&&`, never `;` (it became part of a file name). Inline `cmd` one-liners lose PowerShell's quotes, and a `start /b` child keeps the ssh session open until
  killed. Start processes with `Start-Process -PassThru` and stop them by that id, never by name: Herbert's own copy
  and browser run there too. The worktree may have been removed by another session: check it exists first. Make the
  script remove an existing worktree before `git worktree add`, and print the commit it builds (`git log --oneline
  -1`): when `add` fails on a leftover folder, the build silently tests the old branch (pictures from it misled #75).
  A copy started through master-control's `exec` runs elevated in the desktop session: ssh's `taskkill` can't stop
  it, and its locked `dist\` .exe leaves the worktree half-deleted. Stop it with master-control's `exec` (`taskkill
  /PID n`) too. `--capture` over ssh leaves combo boxes blank (even S7's): check their values with master-control's
  `screenshot` of the window instead (#76).
- Windows: `powershell -ExecutionPolicy Bypass -File windows\scripts\build.ps1` (copies the .exe to `dist\` and the Desktop). Scripts are blocked on windows, so every `.ps1` runs with `-ExecutionPolicy Bypass`, which lifts the block for that one run without changing the system setting. `--capture
  FILE.bmp` draws the window with WM_PRINT, which works over ssh (session 0) where PrintWindow is black; the frame
  comes out black there. Win32 constants missing from the enabled `windows` crate features can be used by value.
  `IsWindowVisible` on a child is false until its top-level window first shows: keep layout state (such as whether
  the sidebar is shown) in the app's own flags.
  Win32 controls notify synchronously (setting a ListView checkbox sends LVN_ITEMCHANGED before the call returns),
  so per-window state in a `RefCell` must be borrowed with `try_borrow_mut` in notification handlers; a second
  `borrow_mut` panics, and a `windows_subsystem` app then just vanishes with no message.
  Paths with "Herb's" in them don't survive `ssh windows '…'`: the apostrophe ends the shell's quoting and
  vanishes, so a command made a new "Herbs Download Manager" folder. Put such steps in a `.ps1` (`Join-Path
  $env:LOCALAPPDATA "Herb's Download Manager"`), copy it over with scp and run that.
  A control whose state follows another's text (enabled when the link is a stream) must also be set where the form
  sets that text: the EN_CHANGE sent inside `with_form` finds the form borrowed and is dropped, so a prefilled link
  left "Only the sound" dimmed (#78).
  Don't let sibling controls overlap (a tab control's body under its pages): WM_PRINT paints siblings in its own
  order, so the pictures showed empty pages. Make the tab control just the strip and put the pages below it.

## The browser extension

- Its background script is tested in Node with a stand-in browser API: `node --test tests/extension`
  (`tests/extension/harness.js` loads the real `background.js` and `naming.js`; `load({tabs, fetch, native, script})`
  sets what the browser, the network, `hdm` and the page answer). `scripts/test.sh` runs it, so the check does.
  Test there, not with throwaway scripts: a new detection, ordering or naming rule gets its case in
  `background.test.js`. Tests stay out of `extension/`, which ships whole (every file in it is embedded, #83).

- Every change to `extension/` raises `version` in `extension/manifest.json` (1.1 → 1.2): each install step copies a
  version Herbert hasn't got yet into his Downloads folder as "Herb's DM extension <version>" (his request,
  2026-11-10), and the app's Show Extension Folder copy is rewritten from the same files.
- **Right after merging a change to `extension/`, run `sh scripts/extension-everywhere.sh`**: it copies the merged
  version from origin/main into the Downloads folder of the Mac, linux and windows. Merges don't install,
  so 1.5–1.10 never reached the Mac's Downloads until he asked where they were (bugs.md, 2026-11-10).

## Feature tests

`tests/features.scenario` is run by each app through its own command code (`core/src/scenario.rs` has the language,
and `DirectHost`, which runs it against the core alone in `cargo test`: a new `do` command needs it too):
`macos/scripts/features.sh` (part of `scripts/test.sh`), `sh linux/scripts/features.sh` after `linux/scripts/build.sh
--install`, `powershell -ExecutionPolicy Bypass -File windows\scripts\features.ps1` after `windows\scripts\build.ps1`. A new feature gets
its steps there; each feature downloads into its own folder, and waits for its downloads to finish, because a link to a
file another unfinished download fetches becomes its mirror. A host step refreshes the list before selecting rows: the list is
refreshed only twice a second, and a command acts on the rows as the list last saw them (Resume once skipped a row it
still thought was running).
On Linux, "the app wrote no report" with "unable to lock lockfile …/hdm-features.lock" in
`~/.cache/hdm-features/session.log` means another session's feature run holds the test compositor: wait for it, then
reinstall your build (theirs may have replaced it) and run again.

## Writing tests (learned the hard way)

- `hd-mock` servers that fake a dying or changed file must also cut responses already under way; otherwise "source
  dies midway" tests pass or fail by timing and never exercise recovery (#7).
- A test that acts once a download reaches N bytes must pick N well before the event it races with: progress counts
  only bytes written in `write_buffer` pieces, so the counter can reach N just as the event fires.
- Don't bound the bytes a server sent tightly when checking that nothing is fetched twice: data already in flight to
  a connection that was cut short counts as sent. Compare against a full refetch instead.
- Anything measured per tick (speeds, slow connections) must count bytes received, not written (#25).
- A test that moves the scheduler's clock (`advance_clock`) must move it to exact times worked out from
  `scheduler_now()`, never by fixed amounts: real time keeps running under the offset, so a test that starts late in a
  minute overshoots the next event (the scheduler test failed on Linux, #59).
- A core test that removes downloads with their files must give its Manager a Trash folder of its own
  (`use_trash_folder`), or their files go to Herbert's real Trash (one test's empty pieces folder did, #87).
- A test that fails now and then: run it 30–60 times on the machine where it fails to get the real error first
  (Windows "access denied" on a name being deleted, #30).
