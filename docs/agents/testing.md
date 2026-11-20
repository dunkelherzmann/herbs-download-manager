# Feature tests and writing tests

Part of this project's agent notes (`CLAUDE.md` points here); these rules are as binding as that file.

## Feature tests

`tests/features.scenario` is run by each app through its own command code (`core/src/scenario.rs` has the language,
and `DirectHost`, which runs it against the core alone in `cargo test`: a new `do` command needs it too):
`macos/scripts/features.sh` (part of `scripts/test.sh`), `sh linux/scripts/features.sh` after `linux/scripts/build.sh
--install`, `powershell -ExecutionPolicy Bypass -File windows\scripts\features.ps1` after `windows\scripts\build.ps1`. A new feature gets
its steps there; each feature downloads into its own folder, and waits for its downloads to finish, because a link to a
file another unfinished download fetches becomes its mirror. A host step refreshes the list before selecting rows: the list is
refreshed only twice a second, and a command acts on the rows as the list last saw them (Resume once skipped a row it
still thought was running).
On Linux, "the app wrote no report" with nothing in the session log but KWin's lines means another copy of the app is
running in the session (one a test Edge's `hdm` opened): the new one hands over to it and quits. Stop that test copy first.
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
- Never compare the engine's own count (connections receiving, bytes kept) with the server's limit while responses
  can finish: the client hangs up about 0.2 ms before it lowers its count, and the server frees the slot on its
  last byte, so a slow CI runner sees one connection too many (#103). Check limits where nothing can finish (a server
  too slow to end a range), and trace both sides with env-gated `eprintln!` before guessing which one is wrong.
- A Manager download retries as Settings say (S6, 5 retries up to 60 s apart), not as `NewDownload.retries`: to
  make one reach Failed in a test, give it failures retrying can't fix (404, a sign-in it lacks), or it stays
  Connecting past the test's wait (#108).
- Anything measured per tick (speeds, slow connections) must count bytes received, not written (#25).
- A test that moves the scheduler's clock (`advance_clock`) must move it to exact times worked out from
  `scheduler_now()`, never by fixed amounts: real time keeps running under the offset, so a test that starts late in a
  minute overshoots the next event (the scheduler test failed on Linux, #59).
- A core test that removes downloads with their files must give its Manager a Trash folder of its own
  (`use_trash_folder`), or their files go to Herbert's real Trash (one test's empty pieces folder did, #87).
- A test that fails now and then: run it 30–60 times on the machine where it fails to get the real error first
  (Windows "access denied" on a name being deleted, #30).
- **When a stream fails to join,** rejoin the kept `.hdparts` pieces with a scratch test (`Joiner` on the folder)
  and diff `ffprobe` packet times against the raw pieces: decode-only checks miss fragment drift and a wrong
  VP9 pick.
- **A session sign-in remembered from an earlier browser add** overrides a download's Dress Referer for that host
  until the app restarts: when testing header changes, add with a fresh host or check the vault's session entries.
