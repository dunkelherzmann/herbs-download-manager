# Notes for agents working on Herb's Download Manager

Read `~/.agents/library/herberts-preferences/SKILL.md` first (Herbert's global CLAUDE.md says so). `SPECIFICATION.md`
is the product, `macos/pending.md` the Mac build order, `enhancements.md` and `bugs.md` the record of requests.

## Checking and merging

- The repository is public (since 2026-11-10). Releases: `scripts/release.sh` builds and checks the four files from
  origin/main (HDM_RELEASE_REF=origin/BRANCH to try a branch), and `scripts/release.sh --publish` tags v<version> and
  publishes the release, marked Latest (a repo with only pre-releases shows none on its front page). On Linux, run a command in a build folder with `flatpak build build CMD`, not
  `flatpak-builder --run`: since fusermount3 3.18 on the Linux machine, `--run` fails with "Failure spawning rofiles-fuse"
  (2026-11-20; it also exits 1 even for `true`).

- GitHub Actions checks every pull request again (free since the repo went public, 2026-11-10;
  `.github/workflows/build.yml`). Before merging, also check a pushed branch with `scripts/check.sh`, which runs the Mac build and tests here and `cargo test --workspace` on the Linux and Windows machines (their ssh names come from the untracked `scripts/hosts.env`; see `scripts/hosts.env.example`), then merge.
  Never edit `scripts/check.sh` (or any shell script) while a background run of it is in flight: sh reads the file
  as it goes, and an edit made a failed Windows run report as passed.
  Merge only on its exit status 0 and a `passed` line for every machine: never judge it through `| tail`, which
  returns tail's status (#89 merged with Linux and Windows failing, 2026-11-10).
  Merge only when `gh pr checks` shows that every GitHub Actions job passed too: a failing Actions job was missed
  on #101 (2026-11-10).
  Every merge to main raises the version (`scripts/version.sh`), a docs-only one too: merge bookkeeping PRs (bugs.md
  ticks, notes) before building the installs, or the installed copies are one behind main (2026-11-17). A build from
  anything but main shows main's number plus `+branch.N`.
  After `gh pr merge`, confirm the PR says MERGED before removing its worktree or branch: a merge refused for a
  conflict, then the branch deleted, closed #100 unmerged and the installs built main without it (2026-11-10).
- Other sessions often work in this same folder: do your own work in a worktree of your own from the start (`git
  worktree add ~/Projects/wt-hdm-<topic> -b <branch> origin/main`), and run `scripts/check.sh` there. In the shared
  checkout, another session switched the branch under a running task: a commit landed on local main and the check
  tested the other session's commit (#64).
- Never run `cargo fmt`: there is no `rustfmt.toml` and the code isn't rustfmt-formatted (about 120-column lines);
  it rewrites the whole workspace.
- These rules are hard stops too: `scripts/guard-bash.mjs`, a PreToolUse hook in `.claude/settings.json`, blocks
  `cargo fmt`, `scripts/check.sh` piped into tail or head, `pkill -f` inside `ssh host '…'`, commits, PR merges and
  branch switches (other than back to `main`) in the shared checkout, and ssh commands with an unquoted "Herb's"
  (tests in `tests/guard`, run by `scripts/test.sh`). It applies to sessions opened in this folder or a worktree of
  it; a session started elsewhere (`~/Projects`) doesn't load it. `scripts/check.sh` also fails a branch that
  changed `extension/` without raising the manifest's version.

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
- A web view the app runs out of sight (the page player, YouTube's page; WKWebView, WebKitGTK, WebView2) must sit in
  a shown but invisible window (off every screen, transparent, ignoring the mouse): outside one its video never
  really plays, YouTube gives only a cold token and refuses the stream after ~5 MB, and timers slow down. On the Mac
  it's also named as Safari, or Vimeo sends no player. Check `currentTime` moves before trusting a hidden run (2026-11-25).

## More notes (read when the step needs them)

- Building, running or testing the Linux or Windows app (Flatpak, cargo in the SDK, `.ps1` over ssh, Win32 traps,
  the installer, stopping a test copy): read `docs/agents/linux-windows.md` first.
- Any change to `extension/` or a test in a browser (Node harness, popup preview, headless and signed-in Edge,
  embedded frames, Downloads copies): read `docs/agents/extension.md` first.
- Adding a feature or writing or fixing a test (`tests/features.scenario`, hd-mock, races, clocks, Trash, flaky
  tests, stream joins): read `docs/agents/testing.md` first.
