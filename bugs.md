# Bugs

Every bug reported for Herb's Download Manager, oldest first. New reports are added at the end and nothing is removed.
A box is ticked once the fix has shipped. A bug that affects several platforms has a box for each, and its main box is ticked when they're all done.
Items marked *found in testing* were found while testing other work, not reported by a user.

No bugs reported yet.

- [ ] **Core test `downloads_of_one_file_starting_together_get_different_names` fails now and then** (*found in testing*, 2026-11-09). About one run in three of `cargo test -p hd-core --test engine` panics at `core/tests/engine.rs:418`; the others pass. All platforms (shared core).
