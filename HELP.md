# Herb's Download Manager Help

Herb's Download Manager (Herb's DM) downloads files over many connections at once, from one server or from several
mirrors of the same file, and pauses and resumes without losing what it has. It also downloads streamed video and
sound, finds the files on a web page, and works with your browser through its extension. It runs on macOS, Linux and
Windows and looks and works the same on each.

Shortcuts are written for the Mac. On Linux and Windows, Ctrl takes the place of ⌘ (⇧⌘N is Ctrl+Shift+N).

## The window

- **Sidebar:** All, Downloading, Paused, Finished and Unfinished, each with its count. Open one to narrow it to a kind
  of file (Compressed, Documents, Music, Pictures, Video, Programs, Other). Show or hide it with View ▸ Show Sidebar
  (⌃⌘S).
- **List:** one row a download, with its name, note, status, size, speed, time left, connections, mirrors and date.
  Click a column's title to sort by it. View ▸ Columns chooses which ones show.
- **Toolbar:** New Download, Resume, Pause, Start Queue, Stop Queue, Stop All, Delete, Settings, Keep on Top (📌) and
  Search (🔍). Every button says what it does when you hover over it.
- **Main Menu (☰):** every menu of the app (File, Edit, Tasks, View, Window, Help), then Settings, About and Quit, so
  no command needs a trip to the top of the screen.
- **Footer:** how many downloads are selected and active, and whether the queue is running.

## Adding downloads

- **New Download (⌘N):** paste a link, choose the folder and the number of connections, and add mirrors if you have
  them. Download starts it now; Add puts it in the queue. It starts downloading while the window is still open, so no
  time is lost.
- **Several links at once:** Add from Clipboard (⇧⌘V, or ⌘V in the main window), Import Links (⇧⌘O) from a text, HTML,
  RTF or CSV file, or drop links, text or files on the window or the Dock icon. They open **Add Links**, where you tick
  the ones you want. Links to the same file become one download with mirrors.
- **Batch Download (⇧⌘N):** a link with a numbered or lettered part (`file*.zip` from 1 to 20) becomes one download
  for each.
- **A web page:** give a page's address instead of a file's and the app looks through it: embedded players,
  hidden addresses, streams and files. Add Links then lists what it found, each checked already with its name and
  size.
- **A name already in use:** the app asks whether to skip the new link, replace the existing file, or keep both.
  Settings ▸ When a file exists can answer once for all.

## Mirrors and connections

A download can come from several **sources**: its link and any mirrors serving the same file. Connections are spread
over the sources that work, so one file comes from several servers at once.

- Add mirrors before (New Download's Mirrors field), during or after a download with **Add Mirror…** (⌥⌘M), or drop
  links onto a download in the list.
- A mirror that serves another file, has no resume support or keeps failing is set aside on its own, and its share
  goes to the others. One good mirror is enough: a link that's gone, wants a sign-in or never answers gives way to it,
  and that mirror becomes the main link. **Make Main Link** in the download window's Mirrors tab does it by hand. **Copy Link** there (or ⌘C / Ctrl+C)
  copies a source's address.
- Each download uses up to 32 connections (Settings ▸ Connections per download). A connection that is much slower than
  the others hands its part to a faster source.

## The download window

Double-click a download, or press ⌘O. It shows the file as segments, its speed and time left, and four tabs:

- **Info:** link, folder, size, whether it can resume, dates and checksum.
- **Connections:** each connection's server, part, speed and state; **+** and **−** add or remove one now.
- **Mirrors:** each source with its state; turn one off, add one, remove one, or make one the main link.
- **Options:** connections, a speed limit for this download, and a checksum to verify at the end (MD5, SHA-1,
  SHA-256 or SHA-512).

## Pause, resume and restart

**Pause** (⌘.) keeps everything downloaded so far; **Resume** (⌘R) carries on, even after quitting the app or a
restart of the computer. Downloads that were running when you quit come back paused and resume at the next launch.
**Restart** starts over from every source. When a file changed on the server, the app notices and says so instead of
mixing two versions.

## Queue and scheduler

**Start Queue** (⇧⌘R) starts waiting downloads, a few at a time (Settings ▸ Downloads at once); **Stop Queue** lets the
running ones finish. In Settings ▸ Scheduler the queue can start and stop at set times on chosen days, and when it's
done the app can quit, put the computer to sleep or shut it down, after a 60-second countdown you can cancel.

## Finished downloads

Open a file, Show in Finder (Show in Folder), **Rename…**, **Move To…**, or add a **Note…** that shows in its own
column and is found by the search. **Delete** asks whether to remove it from the list only or also move its file to
the Trash, where it can be put back.

## Streamed video and sound

The app downloads HLS (`.m3u8`) and DASH (`.mpd`) streams and joins their pieces into one MP4 without re-encoding, so
the file keeps the original quality.

- **Quality, sound and subtitles:** Settings sets the defaults (Highest, the stream's own sound, no subtitles); the
  browser extension lets you choose for each stream.
- **Sound only** saves just the sound track (`.m4a`, or `.mp3`).
- **A section of a long video:** **Download Section…** in the extension asks From and To (hours, minutes and seconds)
  and downloads only that part, cut on the stream's own piece boundaries.
- **Live streams** are recorded until you pause them. Protected (DRM) video can't be saved.

## The browser extension

The extension (Chrome, Edge, Brave, Vivaldi and Firefox) hands your browser's downloads to the app and lists the
videos and sounds a page plays. Settings ▸ Browser extension ▸ **Show Extension Folder** shows it, with how to load it.
Open the app once before using it, so it can register itself with your browsers.

- **Take downloads** (the switch at the top of its list) sends files the browser would download to the app.
- Its list shows each video, stream, sound and file the page has, with a picture and its length. **Download** starts
  it in the app; **▾** offers Add to Queue, Sound Only, Download Section… and Copy Address; **✏** names it first.
- The bar at the bottom has **Naming Rules** (how each site's videos are named and saved), **No video?** (loads the
  page again so its player fetches the video again), **Refresh** (the video playing now), **Find Files on This Page**,
  the list's order and the side panel.
- Right-click a link, picture, video or page for **Download Now with Herb's Download Manager** and **Add to Queue**.

## Sign-ins

When a server asks for a user name and password, the app asks for them and can remember them for the site. Saved
sign-ins live in the app's own store, encrypted with an **Administrator Password** you choose (Settings ▸ Sign-ins ▸
Manage…), never in the system's keychain.

## Command line and MCP server

`hdm` controls the running app from a terminal or a script, and serves the same commands to AI assistants over MCP:

```sh
hdm add LINK… [--folder FOLDER] [--queue] [--quality 1080] [--sound en] [--sound-only] [--from 0:12:00 --to 0:20:00]
hdm list [--filter downloading] [--search TEXT] [--json]
hdm pause ID…        hdm resume ID…        hdm remove ID… [--delete-files]
hdm scan PAGE [--add]
```

## When something goes wrong

- **"The stream's link expired":** open the video's page again and add it anew from the extension.
- **"403 Forbidden" on a stream:** add it again from the extension on the same computer, after playing the video, so
  the app asks for it as your browser did.
- **"The server asks for a user name and password":** enter them when asked, or add them under Settings ▸ Sign-ins.
- **macOS won't open the app:** it isn't notarized by Apple. Right-click it and choose Open, or click Open Anyway in
  System Settings ▸ Privacy & Security.

Herb's Download Manager is a testing app, shared as is: no support is promised or given. What's reported and what's
done is tracked in [bugs.md](bugs.md) and [enhancements.md](enhancements.md); how everything works in detail is in
[SPECIFICATION.md](SPECIFICATION.md).
