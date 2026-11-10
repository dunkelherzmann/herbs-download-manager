# Herb's Download Manager

A download manager for macOS, Linux and Windows. It downloads one file over many connections at once, from one
server or from several mirrors of the same file, and pauses and resumes without losing what it has.

- **Many connections per file**, split dynamically: a free connection takes half of the largest piece still to fetch.
- **Mirrors**: give other servers that hold the same file, before, during or after adding a download, and the
  connections are spread over all of them.
- **Pause and resume**, across quits, crashes and restarts.
- A simple window: menu bar, toolbar, status filters, the download list.
- **A command line and an MCP server**, `hdm`, so scripts and AI assistants can list, add, pause, resume and remove
  downloads while the app runs.

How it works is described in [SPECIFICATION.md](SPECIFICATION.md).

## Status

Early development: the macOS version is being built first (it opens an empty window so far), then Linux and
Windows. What each version still lacks is in [macos/pending.md](macos/pending.md), [linux/pending.md](linux/pending.md) and
[windows/pending.md](windows/pending.md).

## Command line and MCP server

Each app carries `hdm` (SPECIFICATION.md §12). While the app is open:

```sh
hdm add https://example.com/big.iso --folder ~/Downloads/isos
hdm list
hdm pause 3
hdm note 3 for the old laptop
```

Where it is: `/Applications/Herb's Download Manager.app/Contents/MacOS/hdm` on a Mac, `hdm.exe` beside the app on
Windows, and `flatpak run --command=hdm org.herbsdownloadmanager.app` on Linux.

For an AI assistant, `hdm mcp` is an MCP server on standard input and output. In Claude Code, for example:

```sh
claude mcp add herbs-download-manager -- "/Applications/Herb's Download Manager.app/Contents/MacOS/hdm" mcp
```

## Building

On a Mac with Apple silicon, the Xcode Command Line Tools and Rust (rustup):

```sh
macos/scripts/build.sh --install   # builds dist/Herb's Download Manager.app and copies it to /Applications
scripts/test.sh                    # Rust tests, then the app's self-check
cargo run -p hd-mock               # local test server: a 64 MiB file and two mirrors
scripts/check.sh                   # a pushed branch on the Mac, Linux and Windows machines (in place of CI)
```

## Support

Herb's Download Manager is a testing app, shared publicly so others can benefit from it. It is provided as is: no
support is promised or given.

What has been reported and what's done is tracked in the open: [bugs.md](bugs.md) and [enhancements.md](enhancements.md).

## Licence

MIT: use it, change it, share it. See [LICENSE](LICENSE). By dunkelherzmann (github.com/dunkelherzmann).
