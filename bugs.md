# Bugs

Every bug reported for Herb's Download Manager, oldest first. New reports are added at the end and nothing is removed.
A box is ticked once the fix has shipped. A bug that affects several platforms has a box for each, and its main box is ticked when they're all done.
Items marked *found in testing* were found while testing other work, not reported by a user.

No bugs reported yet.

- [x] **Core test `downloads_of_one_file_starting_together_get_different_names` fails now and then** (*found in testing*, 2026-11-09). About one run in three of `cargo test -p hd-core --test engine` panics at `core/tests/engine.rs:418`; the others pass. All platforms (shared core). Two name races: two downloads picking the same name, fixed in #21 (the name is claimed by creating its `.hdpart` exclusively); and, on Windows only (1 run in 6), "Can't create the file … Access denied" when one download claimed a name another was just deleting or renaming, which Windows keeps "delete pending" for a moment. That now counts as busy and is retried, and an access denied that lasts is still reported. Real downloads of one file started together could fail the same way. Fixed in #30; 60 runs in a row pass on Windows.
- [x] **A slow mirror kept its connections when the others ran under about 2.5 MB/s each** (*found in testing*, 2026-11-09). The check for slow connections compared the bytes each one had written in the last tenth of a second, but writes come in 256 KiB pieces, so at those speeds most ticks showed nothing written anywhere and no connection ever counted as slow; the download then waited minutes on the slow mirror's last range. Seen as `slow_connections_hand_their_ranges_to_a_faster_source` failing on the Windows CI runner. It now compares bytes received, and the test runs the fast server at 1 MiB/s per connection so it covers those speeds. Fixed in the shared core, so for all platforms, in #25.
- [x] **The window's edges looked unfinished** (reported 2026-11-09). Where the sidebar met the title bar the lines were faint and didn't line up: the sidebar ran up under the title bar, macOS blended its edge into it, and the line under the title bar stopped at the sidebar. The footer had no edge at all. Now the title bar is one solid band with a sharp line under it across the whole window, the sidebar's divider starts on that line, and the footer is a bar of its own with a line above it. Fixed for the Mac in #28.
  - [x] Mac
  - [x] Linux (checked from a picture: libadwaita's header, sidebar divider and footer bar already meet cleanly)
  - [x] Windows (#48: full-width top and footer bands with one-pixel lines, and a sidebar divider; the list lost its sunken 3D edge)
- [x] **The Windows app could crash when the list was rebuilt with downloads selected** (*found in testing*, 2026-11-09). Restoring the selection made the list report the change at once, which refreshed the list again while its state was in use, and the app ended. Found by the feature tests (#49); the main window now skips such a re-entrant refresh, as its dialogs already did.
  - [x] Windows
- [x] **Quitting on Linux and Windows didn't ask while downloads ran, and on Linux didn't pause them for the next launch** (*found in testing*, 2026-11-09). §10 asks "Downloads are running. Pause them and quit?" (Cancel the default) and starts them again at the next launch; only the Mac did. On Linux, quitting also skipped the step that marks running downloads to start again and saves them. Both now ask, and Linux quits through the shared core as the others do. Fixed in #54.
  - [x] Mac (already)
  - [x] Linux
  - [x] Windows
