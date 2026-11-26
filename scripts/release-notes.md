A download manager for macOS, Linux and Windows. It downloads a file over many connections at once, from one server or from several mirrors, and pauses and resumes without losing what it has. It also has a queue and a scheduler. It joins HLS and DASH streams into one MP4 without re-encoding them, finds the files on a web page, and has a browser extension and a command line and MCP server (`hdm`).

This is a testing app in early development (0.x), shared as is: no support is promised or given. What's reported and what's done is tracked in [bugs.md](https://github.com/dunkelherzmann/herbs-download-manager/blob/main/bugs.md) and [enhancements.md](https://github.com/dunkelherzmann/herbs-download-manager/blob/main/enhancements.md).

## What's new since 0.1.171

- **Paste links to have them scanned,** as JDownloader's link crawler does: ⌘V on the Mac, Ctrl+V on Linux and
  Windows. Every link on the clipboard opens in Add Links, a web page gives way to the files and media on it, and a
  server's folder listing is read with its subfolders. Copied web-page text brings the links behind it.
- **YouTube links pasted into the app** list the video by its title and size and download it, without the browser.
  Other video pages have their player run out of sight when their text names no video, so a streaming page finds its
  stream.
- **Linux and Windows work like the Mac app,** checked feature by feature: the Download window, Settings, Sign-ins,
  Add Links, the clipboard watch, shortcuts, Quit and Pause questions, drag and drop, and on Linux a status-area icon
  while the window is closed.
- **Import Links** reads Word, OpenDocument, web archive and bookmark files on Linux and Windows too.
- **Fixes:** a speed-limited download's count moves while it runs; Windows names the Recycle Bin; Windows' Find bar
  shows; Return presses the default button in Linux's dialogs.
- **Edge installs the extension by itself** from Microsoft Edge Add-ons: the app tells Edge about it at launch, and
  Edge asks to turn it on (on Linux it's on at once). No Developer mode needed in Edge.
- **The extension** (1.37) asks YouTube again when a connection drops mid-download. When Edge holds both the Edge
  Add-ons copy and one loaded unpacked, only one of them takes the downloads.

## Install

**macOS** (15 or later, Apple silicon): download `Herbs-Download-Manager-VERSION-macOS.zip`, open it, and move **Herb's Download Manager** to Applications. It isn't notarized by Apple, so the first time, right-click it and choose **Open**, then **Open** again. If macOS still refuses, go to System Settings ▸ Privacy & Security and click **Open Anyway**.

**Linux** (Flatpak, x86_64): download `Herbs-Download-Manager-VERSION-Linux.flatpak`, then run:

```sh
flatpak install --user Herbs-Download-Manager-VERSION-Linux.flatpak
flatpak run org.herbsdownloadmanager.app
```

This installs the GNOME runtime from Flathub if you don't already have it.

**Windows** (10 or 11, 64-bit): download `Herbs-Download-Manager-VERSION-Windows-Setup.exe` and open it. Until the Windows files are signed, Windows SmartScreen stops the installer the first time with "Windows protected your PC": click **More info**, then **Run anyway**. It installs the app for your account only (no administrator password) and adds it to the Start menu; Settings ▸ Apps removes it.

To use it without installing, download `Herbs-Download-Manager-VERSION-Windows.zip` instead and extract it. Keep `HerbsDownloadManager.exe` and `hdm.exe` in the same folder, because `hdm.exe` is what the browser extension talks to. SmartScreen may stop it the same way.

## Browser extension (EXTENSION)

The extension hands your browser's downloads to the app and lists the media a page plays. It is built into the app: open Settings ▸ Browser extension ▸ **Show Extension Folder**. It is also attached here as `Herbs-DM-extension-EXTENSION.zip`.

- **Edge:** nothing to do. After you open the app once, Edge installs the extension from [Microsoft Edge Add-ons](https://microsoftedge.microsoft.com/addons/detail/claljmehooppmcegokpkiffgknfdknao) at its next start and asks to turn it on.
- **Chrome, Brave, Vivaldi:** open Extensions, turn on Developer mode, click **Load Unpacked**, and choose the folder.
- **Firefox:** open `about:debugging`, then This Firefox ▸ **Load Temporary Add-on**, and choose `manifest.json`.

Open the app once before using the extension, so it can register itself with your browsers.

`SHA256SUMS.txt` has each file's checksum.

## Code signing policy

Free code signing provided by [SignPath.io](https://about.signpath.io), certificate by [SignPath Foundation](https://signpath.org)

It covers the Windows installer and programs, built from the repository by GitHub Actions. Team roles: author, reviewer and approver [dunkelherzmann](https://github.com/dunkelherzmann) (Herbert Cisneros). Privacy: this program will not transfer any information to other networked systems unless specifically requested by the user or the person installing or operating it; it connects only to the addresses you give it, and has no update check, no analytics and no crash reports. The details are in the [README](https://github.com/dunkelherzmann/herbs-download-manager#code-signing-policy).
