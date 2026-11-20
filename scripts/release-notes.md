A download manager for macOS, Linux and Windows. It downloads a file over many connections at once, from one server or from several mirrors, and pauses and resumes without losing what it has. It also has a queue and a scheduler. It joins HLS and DASH streams into one MP4 without re-encoding them, finds the files on a web page, and has a browser extension and a command line and MCP server (`hdm`).

This is a testing app in early development (0.x), shared as is: no support is promised or given. What's reported and what's done is tracked in [bugs.md](https://github.com/dunkelherzmann/herbs-download-manager/blob/main/bugs.md) and [enhancements.md](https://github.com/dunkelherzmann/herbs-download-manager/blob/main/enhancements.md).

## What's new since 0.1.132

- **Windows looks and works like the Mac app:** the title row, toolbar, sidebar and list, light and dark, drag and
  drop, Add Links, Batch Download, the Download window, and a finish notification with **Open** and **Show in
  Folder**. A per-user installer (no administrator password) comes beside the zip, and it can update a running copy.
- **YouTube:** videos, Shorts included, download through the extension, which streams them in YouTube's own page
  while the app joins the pieces.
- **The extension's list** shows a thumbnail for every picture and video, and has **Select All**, **Delete All**,
  **Reload** and a size threshold. A download button sits on the files a page shows, embedded ones included, and finds
  Instagram stories and feed reels.
- **A name in use:** Keep Both names the new file "name 2.ext".
- **Connections and Mirrors** show what a download used, also when it isn't running.
- The extension can also come from Microsoft Edge Add-ons once it's listed there; the app accepts both copies.

## Install

**macOS** (15 or later, Apple silicon): download `Herbs-Download-Manager-VERSION-macOS.zip`, open it, and move **Herb's Download Manager** to Applications. It isn't notarized by Apple, so the first time, right-click it and choose **Open**, then **Open** again. If macOS still refuses, go to System Settings ▸ Privacy & Security and click **Open Anyway**.

**Linux** (Flatpak, x86_64): download `Herbs-Download-Manager-VERSION-Linux.flatpak`, then run:

```sh
flatpak install --user Herbs-Download-Manager-VERSION-Linux.flatpak
flatpak run org.herbsdownloadmanager.app
```

This installs the GNOME runtime from Flathub if you don't already have it.

**Windows** (10 or 11, 64-bit): download `Herbs-Download-Manager-VERSION-Windows-Setup.exe` and open it. The installer isn't signed, so the first time Windows SmartScreen stops it with "Windows protected your PC": click **More info**, then **Run anyway**. It installs the app for your account only (no administrator password) and adds it to the Start menu; Settings ▸ Apps removes it.

To use it without installing, download `Herbs-Download-Manager-VERSION-Windows.zip` instead and extract it. Keep `HerbsDownloadManager.exe` and `hdm.exe` in the same folder, because `hdm.exe` is what the browser extension talks to. SmartScreen may stop it the same way.

## Browser extension (EXTENSION)

The extension hands your browser's downloads to the app and lists the media a page plays. It is built into the app: open Settings ▸ Browser extension ▸ **Show Extension Folder**. It is also attached here as `Herbs-DM-extension-EXTENSION.zip`.

- **Chrome, Edge, Brave, Vivaldi:** open Extensions, turn on Developer mode, click **Load Unpacked**, and choose the folder.
- **Firefox:** open `about:debugging`, then This Firefox ▸ **Load Temporary Add-on**, and choose `manifest.json`.

Open the app once before using the extension, so it can register itself with your browsers.

`SHA256SUMS.txt` has each file's checksum.
