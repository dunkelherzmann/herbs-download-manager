A download manager for macOS, Linux and Windows. It downloads a file over many connections at once, from one server or from several mirrors, and pauses and resumes without losing what it has. It also has a queue and a scheduler. It joins HLS and DASH streams into one MP4 without re-encoding them, finds the files on a web page, and has a browser extension and a command line and MCP server (`hdm`).

This is a testing app in early development (0.x), shared as is: no support is promised or given. What's reported and what's done is tracked in [bugs.md](https://github.com/dunkelherzmann/herbs-download-manager/blob/main/bugs.md) and [enhancements.md](https://github.com/dunkelherzmann/herbs-download-manager/blob/main/enhancements.md).

## What's new since 0.1.98

- **One good mirror replaces a bad link.** Every source is asked at once, a server that never answers is given up
  after 30 seconds, and a link that sends a web page instead of the file is set aside. A mirror that served the file
  becomes the download's main link, and **Make Main Link** in the Mirrors tab does it by hand.
- **Links tied to IPv4 or IPv6.** A stream whose links only work over one address family (as some sites tie them to
  the browser's network) is fetched over that family.
- **Pages scanned more deeply.** The scan opens embedded players, decodes hidden addresses (script escapes, entities,
  percent-encoding, base64, packed scripts), lists DASH streams, and reads pages as a browser would. The extension
  looks through the page as soon as its list opens, with every frame as the browser shows it.
- **A new look for the extension's list:** each item as a card with its picture and length, a Download button with
  more choices beside it, and a bar of tools at the bottom with the extension's version. Its toolbar icon is now as
  big as other extensions' icons.
- **Export Links** writes downloads and their mirrors to a file.

## Install

**macOS** (15 or later, Apple silicon): download `Herbs-Download-Manager-VERSION-macOS.zip`, open it, and move **Herb's Download Manager** to Applications. It isn't notarized by Apple, so the first time, right-click it and choose **Open**, then **Open** again. If macOS still refuses, go to System Settings ▸ Privacy & Security and click **Open Anyway**.

**Linux** (Flatpak, x86_64): download `Herbs-Download-Manager-VERSION-Linux.flatpak`, then run:

```sh
flatpak install --user Herbs-Download-Manager-VERSION-Linux.flatpak
flatpak run org.herbsdownloadmanager.app
```

This installs the GNOME runtime from Flathub if you don't already have it.

**Windows** (10 or 11, 64-bit): download `Herbs-Download-Manager-VERSION-Windows.zip` and extract it. Keep `HerbsDownloadManager.exe` and `hdm.exe` in the same folder, because `hdm.exe` is what the browser extension talks to. It isn't signed, so if SmartScreen stops it, click **More info ▸ Run anyway**.

## Browser extension (EXTENSION)

The extension hands your browser's downloads to the app and lists the media a page plays. It is built into the app: open Settings ▸ Browser extension ▸ **Show Extension Folder**. It is also attached here as `Herbs-DM-extension-EXTENSION.zip`.

- **Chrome, Edge, Brave, Vivaldi:** open Extensions, turn on Developer mode, click **Load Unpacked**, and choose the folder.
- **Firefox:** open `about:debugging`, then This Firefox ▸ **Load Temporary Add-on**, and choose `manifest.json`.

Open the app once before using the extension, so it can register itself with your browsers.

`SHA256SUMS.txt` has each file's checksum.
