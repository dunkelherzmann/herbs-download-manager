# Privacy: Herb's Download Manager browser extension

The Herb's Download Manager extension for Microsoft Edge works with the Herb's Download Manager app on the same
computer. It collects nothing for itself or for anyone else: it has no servers, no accounts, no analytics and no ads.

## What it reads, and why

- **Downloads you start in the browser:** their address, file name and size, so the app can download the file
  instead of the browser. With "Take downloads" off, the extension leaves every download to the browser.
- **The page you're on:** the addresses of the videos, sounds, pictures and files it plays or links to, so its list
  can show them and the download button can appear on them. It reads the page only to find those files.
- **The site's cookies and the request headers for a file** (such as the referring page): handed to the app with that
  file only, because many sites refuse a file to anyone who isn't signed in or didn't come from their page.

## Where it goes

Only to the Herb's Download Manager app on your own computer, through the browser's native messaging, and to the
site the file comes from, as the browser itself would send it. Nothing is sent to the developer or to any third party.

## What it keeps

In the browser's own extension storage, on your computer: its settings, such as whether "Take downloads" is on, the
list's sort order and whether it opens in the side panel. Removing the extension removes them. The app keeps its download list on your
computer too; see its own documentation.

## Source

The extension and the app are open source: github.com/dunkelherzmann/herbs-download-manager. They are provided as is.
