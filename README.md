# Herb's Downloader

A download manager for macOS, Linux and Windows. It downloads one file over many connections at once, from one
server or from several mirrors of the same file, and pauses and resumes without losing what it has.

- **Many connections per file**, split dynamically: a free connection takes half of the largest piece still to fetch.
- **Mirrors**: give other servers that hold the same file, before, during or after adding a download, and the
  connections are spread over all of them.
- **Pause and resume**, across quits, crashes and restarts.
- A simple window: menu bar, toolbar, status filters, the download list.

How it works is described in [SPECIFICATION.md](SPECIFICATION.md).

## Status

Early development: the macOS version is being built first (it opens an empty window so far), then Linux and
Windows. What each version still lacks is in [pending.md](pending.md), [linux/pending.md](linux/pending.md) and
[windows/pending.md](windows/pending.md).

## Building

On a Mac with Apple silicon, the Xcode Command Line Tools and Rust (rustup):

```sh
scripts/build-macos.sh --install   # builds dist/Herb's Downloader.app and copies it to /Applications
scripts/test.sh                    # Rust tests, then the app's self-check
cargo run -p hd-mock               # local test server: a 64 MiB file and two mirrors
```

## Support

Herb's Downloader is a testing app, shared publicly so others can benefit from it. It is provided as is: no
support is promised or given.

What has been reported and what's done is tracked in the open: [bugs.md](bugs.md) and [enhancements.md](enhancements.md).

## Licence

MIT: use it, change it, share it. See [LICENSE](LICENSE). By dunkelherzmann (github.com/dunkelherzmann).
