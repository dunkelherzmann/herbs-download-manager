// Herb's Download Manager for Chrome, Edge, Brave, Vivaldi and Firefox (SPECIFICATION.md §13): hands each new
// download to the app through native messaging (the app's `hdm`), and the browser keeps any download the app
// doesn't take: when the app isn't open, the link doesn't work for it, or the window is private. It also notes the
// videos and sounds each tab plays (§15), which the toolbar button's list (popup.js) adds to the app's queue.

const api = globalThis.browser ?? chrome;
const HOST = "org.herbsdownloadmanager.hdm";
const NAME = "Herb's Download Manager";

async function isOn() {
  const { on } = await api.storage.local.get({ on: true });
  return on;
}

async function showState() {
  const on = await isOn();
  await api.action.setBadgeText({ text: on ? "" : "off" });
  await api.action.setTitle({ title: on ? NAME : `${NAME}: the browser keeps its downloads` });
}

// A short sign on the button when the app didn't take a download, with the reason in its hover text.
async function tell(reason) {
  await api.action.setBadgeText({ text: "!" });
  await api.action.setTitle({ title: `${NAME}: the browser kept the last download. ${reason}` });
  setTimeout(async () => {
    await showState();
    const [tab] = await api.tabs.query({ active: true, currentWindow: true }).catch(() => []);
    if (tab) await countFor(tab.id);
  }, 15000);
}

const web = (url) => /^https?:/i.test(url || "");

// The cookies the browser would send with the link, as a Cookie header.
async function cookiesFor(url) {
  try {
    const all = await api.cookies.getAll({ url });
    return all.map((c) => `${c.name}=${c.value}`).join("; ");
  } catch {
    return "";
  }
}

// Asks the app to download the link; resolves to its answer ({ok, error, message}). `extra`: queue, stream, name.
async function send(url, referer, extra = {}) {
  const message = { command: "add", urls: [url], referer: referer || "", cookies: await cookiesFor(url), check: true, ...extra };
  try {
    return await api.runtime.sendNativeMessage(HOST, message);
  } catch {
    return { ok: false, error: `${NAME} isn't set up for this browser, or isn't installed.` };
  }
}

api.downloads.onCreated.addListener(async (item) => {
  const url = item.finalUrl || item.url;
  if (!(await isOn()) || item.incognito || item.state !== "in_progress" || !web(url)) return;
  // Paused while the app looks at the link, so neither side downloads it twice. One that finished before it could
  // pause (a small file on a fast line) stays with the browser.
  await api.downloads.pause(item.id).catch(() => {});
  const [paused] = await api.downloads.search({ id: item.id });
  // (Firefox reports a paused download as "interrupted".)
  if (!paused || paused.state === "complete") return;
  const answer = await send(url, item.referrer);
  const [now] = await api.downloads.search({ id: item.id });
  if (answer && answer.ok) {
    await api.downloads.cancel(item.id).catch(() => {});
    await api.downloads.erase({ id: item.id }).catch(() => {});
  } else {
    if (now && now.state !== "complete") await api.downloads.resume(item.id).catch(() => {});
    await tell((answer && answer.error) || `${NAME} didn't answer.`);
  }
});

// Right-click a link: Download with Herb's Download Manager.
async function makeMenu() {
  await api.contextMenus.removeAll();
  api.contextMenus.create({ id: "hdm-link", title: `Download with ${NAME}`, contexts: ["link", "image", "video", "audio"] });
}
api.runtime.onInstalled.addListener(() => { makeMenu(); showState(); });
api.runtime.onStartup.addListener(() => { makeMenu(); showState(); });

api.contextMenus.onClicked.addListener(async (info) => {
  const url = info.linkUrl || info.srcUrl;
  if (!web(url)) return;
  const answer = await send(url, info.pageUrl);
  if (!(answer && answer.ok)) await tell((answer && answer.error) || `${NAME} didn't answer.`);
});

// ---- the media a tab plays (§15) ----------------------------------------------------------------------------
//
// What each tab loaded that can be downloaded, in the order seen: stream playlists (HLS .m3u8, DASH .mpd; a master
// playlist hides the quality playlists it lists) and whole video and sound files. Stream pieces, partial ranges a
// player fetches, small files and a page's own short sounds are left out. Kept in session storage, so it survives
// the background script being put to sleep, and forgotten when the tab goes to another page or closes.

const SEGMENT = /\.(ts|m4s|m4f|cmfv|cmfa|aac)$/i;
const MEDIA_EXT = /\.(mp4|m4v|webm|mkv|mov|avi|flv|3gp|ogv|mp3|m4a|ogg|oga|opus|flac|wav)$/i;
const FETCHED_MIN = 1 << 20; // a player's own fetch of a whole file, from 1 MB
const SOUND_MIN = 64 << 10; // shorter sounds a page plays are clicks and chimes
const MAX_ITEMS = 40;

const headerOf = (d, name) => ((d.responseHeaders || []).find((h) => h.name.toLowerCase() === name) || {}).value || "";
const pathOf = (url) => url.split(/[?#]/)[0];

async function itemsOf(tabId) {
  const key = `media-${tabId}`;
  return (await api.storage.session.get({ [key]: [] }))[key];
}

async function setItems(tabId, items) {
  await api.storage.session.set({ [`media-${tabId}`]: items });
  await countFor(tabId, items);
}

// The button's number: what the tab has to download (or "off" while the hand-over is off).
async function countFor(tabId, items) {
  if (!(await isOn())) return;
  items = items || (await itemsOf(tabId));
  const n = items.filter((i) => !i.hidden).length;
  await api.action.setBadgeText({ tabId, text: n ? String(n) : "" }).catch(() => {});
}

// What a response is: {kind: "stream" | "dash" | "video" | "audio", size}, or null.
function kindOf(d) {
  if (d.tabId < 0 || !web(d.url) || d.url.length > 8192) return null;
  if (d.statusCode && (d.statusCode < 200 || d.statusCode > 299)) return null;
  const mime = headerOf(d, "content-type").split(";")[0].trim().toLowerCase();
  const path = pathOf(d.url).toLowerCase();
  if (/\.m3u8$/.test(path) || /mpegurl/.test(mime)) return { kind: "stream", size: 0 };
  if (/\.mpd$/.test(path) || /dash\+xml/.test(mime)) return { kind: "dash", size: 0 };
  if (SEGMENT.test(path) || /^(video\/mp2t|video\/iso\.segment|audio\/iso\.segment)$/.test(mime)) return null;
  const typed = /^(video|audio)\//.test(mime) || mime === "application/ogg";
  const untyped = !mime || /^(application|binary)\/octet-stream$/.test(mime);
  if (!typed && !(MEDIA_EXT.test(path) && untyped)) return null;
  const range = headerOf(d, "content-range");
  const total = /\/\s*(\d+)\s*$/.exec(range);
  const size = total ? Number(total[1]) : d.statusCode === 206 ? 0 : Number(headerOf(d, "content-length")) || 0;
  // A <video>'s or <audio>'s own file comes in ranges; a script's fetch of a range or a small file is a piece.
  if (d.type !== "media" && (d.statusCode === 206 || range || size < FETCHED_MIN)) return null;
  if (d.type === "media" && size > 0 && size < SOUND_MIN) return null;
  const audio = mime.startsWith("audio/") || (!mime.startsWith("video/") && /\.(mp3|m4a|ogg|oga|opus|flac|wav)$/i.test(path));
  return { kind: audio ? "audio" : "video", size, mime };
}

// A playlist's first bytes, fetched again with the site's cookies: a master playlist's quality playlists, if it is one.
async function variantsOf(url) {
  try {
    const r = await fetch(url, { credentials: "include" });
    const text = (await r.text()).slice(0, 256 * 1024);
    if (!text.trimStart().startsWith("#EXTM3U")) return null;
    const lines = text.split(/\r?\n/).map((l) => l.trim());
    const out = [];
    lines.forEach((l, i) => {
      if (l.startsWith("#EXT-X-STREAM-INF") && lines[i + 1] && !lines[i + 1].startsWith("#")) out.push(new URL(lines[i + 1], url).href);
      const uri = /^#EXT-X-MEDIA:.*URI="([^"]+)"/.exec(l);
      if (uri) out.push(new URL(uri[1], url).href);
    });
    return out;
  } catch {
    return [];
  }
}

let noting = Promise.resolve();
function note(d, found) {
  // One at a time: two answers at once would both read the old list.
  noting = noting.then(async () => {
    const items = await itemsOf(d.tabId);
    if (items.some((i) => i.url === d.url)) return;
    const item = { url: d.url, kind: found.kind, size: found.size || 0, mime: found.mime || "", hidden: false };
    if (found.kind === "stream") {
      const variants = await variantsOf(d.url);
      if (variants === null) return; // not a playlist after all
      // A quality playlist of a master already listed is part of it; a master hides the ones seen before it.
      if (items.some((i) => (i.variants || []).includes(d.url))) item.hidden = true;
      item.variants = variants;
      for (const i of items) if (variants.includes(i.url)) i.hidden = true;
    }
    items.push(item);
    await setItems(d.tabId, items.slice(-MAX_ITEMS));
  }).catch(() => {});
}

if (api.webRequest && api.webRequest.onResponseStarted) {
  api.webRequest.onResponseStarted.addListener(
    (d) => {
      const found = kindOf(d);
      if (found) note(d, found);
    },
    { urls: ["<all_urls>"], types: ["xmlhttprequest", "media", "other"] },
    ["responseHeaders"],
  );
}
api.tabs.onRemoved.addListener((id) => api.storage.session.remove(`media-${id}`).catch(() => {}));
// Another page in the tab: what the last one played goes.
api.tabs.onUpdated.addListener((id, change) => {
  if (change.url) noting = noting.then(() => setItems(id, [])).catch(() => {});
});
api.tabs.onActivated.addListener(({ tabId }) => countFor(tabId).catch(() => {}));

// A file name for an item: a stream after its page, a file after its link (or its page, when the link's name says
// nothing).
function nameFor(item, title) {
  const page = (title || "").trim() || "Video";
  if (item.kind === "stream") return `${page}.mp4`;
  const last = decodeURIComponent(pathOf(item.url).split("/").pop() || "");
  if (MEDIA_EXT.test(last)) return null; // the app names it from the link
  const ext = /webm/.test(item.mime) ? "webm" : item.kind === "audio" ? (/mpeg/.test(item.mime) ? "mp3" : "m4a") : "mp4";
  return `${page}.${ext}`;
}

// The popup's questions.
api.runtime.onMessage.addListener((message, sender, reply) => {
  (async () => {
    if (message.type === "state") {
      const items = (await itemsOf(message.tabId)).filter((i) => !i.hidden);
      reply({ on: await isOn(), items });
    } else if (message.type === "toggle") {
      await api.storage.local.set({ on: !(await isOn()) });
      await showState();
      await countFor(message.tabId);
      reply({ on: await isOn() });
    } else if (message.type === "scan") {
      // The app reads the page (with the browser's cookies for it) and checks every file it mentions.
      const tab = await api.tabs.get(message.tabId).catch(() => null);
      if (!tab || !web(tab.url)) {
        reply({ ok: false, error: "This tab has no web page to look through." });
        return;
      }
      try {
        const cookies = await cookiesFor(tab.url);
        reply(await api.runtime.sendNativeMessage(HOST, { command: "scan", urls: [tab.url], cookies, referer: tab.url }));
      } catch {
        reply({ ok: false, error: `${NAME} isn't set up for this browser, or isn't installed.` });
      }
    } else if (message.type === "add") {
      const tab = await api.tabs.get(message.tabId).catch(() => null);
      const item = message.item;
      // A file the app found on the page comes with its name; what the tab played is named here.
      const name = item.name || nameFor(item, tab && tab.title);
      const answer = await send(item.url, tab && tab.url, { queue: true, stream: item.kind === "stream" || item.stream === true, name });
      reply(answer || { ok: false, error: `${NAME} didn't answer.` });
    }
  })();
  return true; // answers later
});
