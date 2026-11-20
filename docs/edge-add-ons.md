# Edge Add-ons listing

What the extension's listing on Microsoft Edge Add-ons says, so each update is submitted with the same text.
`scripts/edge-store-package.sh` builds the package and the logo; the screenshots are the extension's list
(`tests/extension/popup-preview`) beside a short description, light and dark, at 1280 × 800.

- **Name and short description:** from `extension/manifest.json`.
- **Category:** Productivity. **Mature content:** no. **Visibility:** Public. **Markets:** all.
- **Website:** https://github.com/dunkelherzmann/herbs-download-manager
- **Privacy policy:** https://github.com/dunkelherzmann/herbs-download-manager/blob/main/PRIVACY.md
- **Search terms:** download manager, downloader, resume downloads, video downloader, accelerator

## Description

Herb's Download Manager hands the browser's downloads to the Herb's Download Manager app, which fetches each file
over many connections at once, from mirrors when it has them, and picks up where it left off after a dropped
connection or a restart.

This extension needs the Herb's Download Manager app on the same computer (Windows, macOS or Linux). It's free and
open source: github.com/dunkelherzmann/herbs-download-manager/releases. Without the app, the browser keeps its
downloads as usual.

What it does:
- Takes the downloads you start in Edge and hands them to the app, with the site's cookies and referring page, so
  files behind a sign-in download too. Turn "Take downloads" off and Edge keeps them.
- Lists the videos and sounds the page you're on plays, and the files it links to, each with its size and a
  Download button.
- Puts a small download button on the sound, picture or video under the pointer.
- Adds "Download Now" and "Add to Queue" to the right-click menu for links.

Nothing leaves your computer except the requests the browser would make anyway: no accounts, no analytics, no ads.
Provided as is.

## Permissions, and why

| Permission | Why |
|---|---|
| downloads, downloads.ui | to see the downloads you start, hand them to the app, and hide Edge's download bubble for those the app takes |
| nativeMessaging | to talk to the Herb's Download Manager app on the same computer |
| cookies | to give the app the site's cookies for a file, so a file behind a sign-in downloads |
| webRequest | to see which files and streams a page loads, and the headers they were asked with, for the list |
| declarativeNetRequestWithHostAccess | to read a file's first bytes (its length and picture size) with the page's cookies and referring page, as the page would |
| scripting | to find the videos, sounds and files on the page when the list opens |
| contextMenus | for "Download Now" and "Add to Queue" on links |
| storage | to remember the extension's own settings |
| sidePanel | to show the list in Edge's side panel when chosen |
| host permissions (all sites) | downloads and media can come from any site |

## Notes for certification

The extension works with the free Herb's Download Manager app, which must be installed and opened once (it
registers itself with Edge): on Windows, `Herbs-Download-Manager-<version>-Windows.zip` from
github.com/dunkelherzmann/herbs-download-manager/releases/latest; unzip it and run the app. To test: open any page with a direct file link
(a .zip or .pdf), click it: the download appears in the app's window instead of Edge's. Open the extension's list
on a page with a video to see its files with Download buttons. With the app not running, the list says "Herb's Download
Manager isn't running." and Edge keeps its downloads.
