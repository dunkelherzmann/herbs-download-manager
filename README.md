# Herb's Download Manager

A download manager for macOS, Linux and Windows. It downloads one file over many connections at once, from one
server or from several mirrors of the same file, and pauses and resumes without losing what it has.

- **Many connections per file**, split dynamically: a free connection takes half of the largest piece still to fetch.
- **Mirrors**: give other servers that hold the same file, before, during or after adding a download, and the
  connections are spread over all of them.
- **Pause and resume**, across quits, crashes and restarts.
- **A queue and a scheduler**: waiting downloads start oldest first, a few at a time, by hand or at set times.
- **Streamed video and sound** (HLS and DASH), joined into one MP4 by its own joiner, never re-encoded, in the
  quality and sound track you choose. DRM-protected streams are refused.
- **Find the files on a web page**: paste or drop a page's address and pick from what it holds.
- **A browser extension** that hands the browser's downloads to the app and lists the media a page plays.
- A simple window: menu bar, toolbar, status filters, the download list.
- **A command line and an MCP server**, `hdm`, so scripts and AI assistants can list, add, pause, resume and remove
  downloads while the app runs.

How it works is described in [SPECIFICATION.md](SPECIFICATION.md).

## Status

A testing app in active development, working on macOS, Linux and Windows. Prebuilt files for all three systems are on the
[Releases](https://github.com/dunkelherzmann/herbs-download-manager/releases) page; they aren't signed or notarized, and the notes there say how to open
each. Or build it yourself (below). What each version still lacks is in [macos/pending.md](macos/pending.md), [linux/pending.md](linux/pending.md) and
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

## Browser extension

The extension in `extension/` hands Chrome's, Edge's, Brave's, Vivaldi's and Firefox's downloads to the app, with the
browser's cookies for them; the browser keeps any download the app doesn't take (it isn't running, or the link doesn't
work for it). Right-click a link for **Download Now** or **Add to Queue**; the toolbar button turns it off and on.

The app registers itself with the browsers each time it opens. To load the extension, open Settings ▸ Browser
extension ▸ **Show Extension Folder**, then:

- Chrome, Edge, Brave, Vivaldi: Extensions ▸ turn on Developer mode ▸ **Load Unpacked**, and choose that folder.
- Firefox: `about:debugging` ▸ This Firefox ▸ **Load Temporary Add-on**, and choose `manifest.json` in it (until the
  extension is signed by Mozilla, Firefox forgets it when it quits).

Safari isn't supported yet.

## Building

Every version builds from this repository with Rust (rustup).

**macOS** (Apple silicon, the Xcode Command Line Tools):

```sh
macos/scripts/build.sh --install   # builds dist/Herb's Download Manager.app and copies it to /Applications
scripts/test.sh                    # Rust tests, then the app's self-check
cargo run -p hd-mock               # local test server: a 64 MiB file and two mirrors
scripts/check.sh                   # a pushed branch on the Mac, Linux and Windows machines (GitHub Actions checks pull requests too)
```

**Linux** (a Flatpak; needs `org.flatpak.Builder`, the GNOME 49 SDK and its Rust extension:
`flatpak install --user flathub org.flatpak.Builder org.gnome.Sdk//49 org.freedesktop.Sdk.Extension.rust-stable//25.08`):

```sh
sh linux/scripts/build.sh --install   # then: flatpak run org.herbsdownloadmanager.app
```

**Windows** (the Visual Studio Build Tools): one portable `.exe`, put in `dist\` and on the Desktop.

```powershell
powershell -ExecutionPolicy Bypass -File windows\scripts\build.ps1
```

## Support

Herb's Download Manager is a testing app, shared publicly so others can benefit from it. It is provided as is: no
support is promised or given.

What has been reported and what's done is tracked in the open: [bugs.md](bugs.md) and [enhancements.md](enhancements.md).

## Licence

MIT: use it, change it, share it. See [LICENSE](LICENSE). By dunkelherzmann (github.com/dunkelherzmann).
