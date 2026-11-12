A download manager for macOS, Linux and Windows. It downloads a file over many connections at once, from one server or from several mirrors, and pauses and resumes without losing what it has. It also has a queue and a scheduler. It joins HLS and DASH streams into one MP4 without re-encoding them, finds the files on a web page, and has a browser extension and a command line and MCP server (`hdm`).

This is a testing app in early development (0.x), shared as is: no support is promised or given. What's reported and what's done is tracked in [bugs.md](https://github.com/dunkelherzmann/herbs-download-manager/blob/main/bugs.md) and [enhancements.md](https://github.com/dunkelherzmann/herbs-download-manager/blob/main/enhancements.md).

## What's new since 0.1.113

- **Proxies:** the app follows the system's proxy settings, automatic setup scripts (PAC) on the Mac and Windows
  included. On Linux it asks the desktop for each address. SOCKS proxies work too.
- **Streams that refused the app:** a stream's pieces are asked for with the same headers the page's player sent, so
  servers that check them no longer refuse. A stream whose link fails tries its mirrors, and **Make Main Link** works
  for streams.
- **Watch the clipboard for links** (Settings, off by default): a copied link opens Add Links.
- **Keep the computer awake** while downloads run.
- **File names every system takes:** characters and names that macOS, Linux or Windows refuse are replaced before the
  file is made. A download stops when less than 128 MB of disk space is left.
- **Add Links** marks a link that fails as Offline or Needs a sign-in, and leaves it unchecked.
- **Mirrors:** **Copy Link** in the Mirrors tab. On Linux and Windows, links dropped on a download become its
  mirrors, and Windows takes links dragged from a browser.
- **The extension** opens the app in the background when its right-click menu or its Download button is used while
  the app is closed.
- **About** shows the app's version on every platform.

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
