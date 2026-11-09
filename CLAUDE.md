# Notes for agents working on Herb's Download Manager

Read `~/.agents/library/herberts-preferences/SKILL.md` first (Herbert's global CLAUDE.md says so). `SPECIFICATION.md`
is the product, `macos/pending.md` the Mac build order, `enhancements.md` and `bugs.md` the record of requests.

## Checking and merging

- No GitHub CI (the repo is private and Actions minutes cost money): check a pushed branch with `scripts/check.sh`,
  which runs the Mac build and tests here and `cargo test --workspace` on linux and windows, then merge.
- Never run `cargo fmt`: there is no `rustfmt.toml` and the code isn't rustfmt-formatted (about 120-column lines);
  it rewrites the whole workspace.

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
