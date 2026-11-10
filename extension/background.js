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
// Previews already made, by link (or manifest): the newest 60.
const previews = new Map();

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
  // A piece of a file a player fetches by byte range in its address (Facebook's bytestart/byteend) is never a file
  // of its own: listed, it saved fragments without the file's beginning or its sound (bugs.md).
  if (/[?&](bytestart|byteend|range|byterange)=/i.test(d.url)) return null;
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

// A tag's attribute, quoted or not: RESOLUTION=1920x1080, LANGUAGE="de".
const attrOf = (line, name) => (new RegExp(`[:,]${name}=("([^"]*)"|[^,]*)`).exec(line) || [])[2] ?? (new RegExp(`[:,]${name}=([^,]*)`).exec(line) || [])[1];

// A playlist's first bytes, fetched again with the site's cookies: a master playlist's quality and sound playlists
// (`urls`, to hide them from the list), its qualities (the most bits a second for each picture size, largest first,
// HDR left out when a standard version exists) and its sound languages. `null` when it isn't a playlist.
async function variantsOf(url) {
  try {
    const r = await fetch(url, { credentials: "include" });
    const text = (await r.text()).slice(0, 256 * 1024);
    if (!text.trimStart().startsWith("#EXTM3U")) return null;
    const lines = text.split(/\r?\n/).map((l) => l.trim());
    const urls = [];
    const variants = [];
    const sounds = [];
    lines.forEach((l, i) => {
      if (l.startsWith("#EXT-X-STREAM-INF") && lines[i + 1] && !lines[i + 1].startsWith("#")) {
        urls.push(new URL(lines[i + 1], url).href);
        const size = /^(\d+)[xX](\d+)$/.exec(attrOf(l, "RESOLUTION") || "");
        variants.push({
          lines: size ? Math.min(Number(size[1]), Number(size[2])) : 0,
          bandwidth: Number(attrOf(l, "BANDWIDTH")) || 0,
          hdr: /^(PQ|HLG)$/.test(attrOf(l, "VIDEO-RANGE") || ""),
        });
      }
      const uri = /^#EXT-X-MEDIA:.*URI="([^"]+)"/.exec(l);
      if (uri) urls.push(new URL(uri[1], url).href);
      const language = l.startsWith("#EXT-X-MEDIA:") && attrOf(l, "TYPE") === "AUDIO" && uri && attrOf(l, "LANGUAGE");
      if (language && !sounds.some((s) => s.language === language)) sounds.push({ language, name: attrOf(l, "NAME") || language });
    });
    const standard = variants.filter((v) => !v.hdr);
    const qualities = [];
    for (const v of (standard.length ? standard : variants).filter((v) => v.lines).sort((a, b) => b.lines - a.lines || b.bandwidth - a.bandwidth)) {
      if (!qualities.some((q) => q.lines === v.lines)) qualities.push({ lines: v.lines, bandwidth: v.bandwidth });
    }
    return { urls, qualities, sounds };
  } catch {
    return { urls: [], qualities: [], sounds: [] };
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
      const found = await variantsOf(d.url);
      if (found === null) return; // not a playlist after all
      const variants = found.urls;
      // A quality playlist of a master already listed is part of it; a master hides the ones seen before it.
      if (items.some((i) => (i.variants || []).includes(d.url))) item.hidden = true;
      item.variants = variants;
      item.qualities = found.qualities;
      item.sounds = found.sounds;
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

// DASH manifests a page carries in its data instead of loading them (Facebook's reels: "dash_manifest" in the page's
// JSON): read from the tab's scripts when the list opens, each listed once as a video with its sound.
function manifestsInPage() {
  const found = [];
  const seen = new Set();
  const keep = (xml) => {
    const text = xml.trim();
    if (!text.includes("<MPD") || text.length > 2 * 1024 * 1024) return;
    const key = text.replace(/\s+/g, "").slice(0, 4000);
    if (seen.has(key)) return;
    seen.add(key);
    found.push(text);
  };
  for (const script of document.scripts) {
    const t = script.textContent || "";
    if (!t.includes("MPD")) continue;
    // In JSON: a string holding the manifest, its < written as \u003C.
    for (const m of t.matchAll(/"((?:[^"\\]|\\.)*?(?:\\u003C|<)MPD(?:[^"\\]|\\.)*?)"/g)) {
      try { keep(JSON.parse(`"${m[1]}"`)); } catch {}
    }
    // As it is.
    for (const m of t.matchAll(/<\?xml[^>]*>\s*<MPD[\s\S]*?<\/MPD>|<MPD[\s\S]*?<\/MPD>/g)) keep(m[0]);
    if (found.length >= 20) break;
  }
  return found;
}

// What a manifest says about the video: its length and best picture.
function describeManifest(xml) {
  const duration = /mediaPresentationDuration="PT(?:(\d+)H)?(?:(\d+)M)?(?:([\d.]+)S)?"/.exec(xml);
  const seconds = duration ? (Number(duration[1] || 0) * 3600 + Number(duration[2] || 0) * 60 + Number(duration[3] || 0)) : 0;
  const heights = [...xml.matchAll(/<Representation[^>]*\sheight="(\d+)"/g)].map((m) => Number(m[1]));
  const widths = [...xml.matchAll(/<Representation[^>]*\swidth="(\d+)"/g)].map((m) => Number(m[1]));
  const best = heights.length ? Math.max(...heights.map((h, i) => Math.min(h, widths[i] || h))) : 0;
  return { seconds, quality: best ? `${best}p` : "", sound: /contentType="audio"|mimeType="audio\//.test(xml), protected: /<ContentProtection/.test(xml) };
}

async function pageManifests(tabId) {
  try {
    const [result] = await api.scripting.executeScript({ target: { tabId }, func: manifestsInPage });
    return (result && result.result) || [];
  } catch {
    return []; // a page the extension may not read (the browser's own pages)
  }
}

// A file name for an item: a stream after its page, a file after its link (or its page, when the link's name says
// nothing).
function nameFor(item, title) {
  const page = (title || "").trim() || "Video";
  if (["stream", "dash", "page-dash"].includes(item.kind)) return `${page}.mp4`;
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
      for (const manifest of await pageManifests(message.tabId)) {
        const about = describeManifest(manifest);
        if (!about.protected) items.unshift({ kind: "page-dash", manifest, url: "", size: 0, ...about });
      }
      reply({ on: await isOn(), items });
    } else if (message.type === "preview") {
      // A stream's first picture and length (§15), made by hdm with the browser's cookies; kept while the browser
      // runs, so the list shows it at once next time.
      const item = message.item;
      const key = item.url || item.manifest;
      if (previews.has(key)) {
        reply(previews.get(key));
        return;
      }
      const tab = await api.tabs.get(message.tabId).catch(() => null);
      const page = tab && web(tab.url) ? tab.url : "";
      const request = item.kind === "page-dash"
        ? { command: "preview", manifest: item.manifest, referer: page, cookies: page ? await cookiesFor(page) : "" }
        : { command: "preview", urls: [item.url], referer: page, cookies: await cookiesFor(item.url) };
      let answer;
      try {
        answer = await api.runtime.sendNativeMessage(HOST, request);
      } catch {
        answer = { ok: false, error: `${NAME} isn't set up for this browser, or isn't installed.` };
      }
      if (answer && answer.ok) {
        previews.set(key, answer);
        if (previews.size > 60) previews.delete(previews.keys().next().value);
      }
      reply(answer || { ok: false });
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
    } else if (message.type === "open") {
      // "Open and Download": the app is opened by hdm, which waits until it answers.
      try {
        reply(await api.runtime.sendNativeMessage(HOST, { command: "open" }));
      } catch {
        reply({ ok: false, error: `${NAME} isn't set up for this browser, or isn't installed.` });
      }
    } else if (message.type === "add") {
      const tab = await api.tabs.get(message.tabId).catch(() => null);
      const item = message.item;
      // A file the app found on the page comes with its name; what the tab played is named here.
      const name = item.name || nameFor(item, tab && tab.title);
      // Added to the queue; right after opening the app ("Open and Download"), started at once.
      const options = { queue: !message.start, stream: ["stream", "dash", "page-dash"].includes(item.kind) || item.stream === true, name };
      // A stream's quality and sound chosen in the list (S13, S14 otherwise).
      if (message.quality) options.quality = message.quality;
      if (message.sound !== undefined) options.sound = message.sound;
      // Only its sound, saved as it is (.m4a, or .mp3).
      if (message.soundOnly && options.stream) options.sound_only = true;
      let answer;
      if (item.kind === "page-dash") {
        // A manifest from the page itself: the app keeps it and downloads the video and its sound from it.
        try {
          answer = await api.runtime.sendNativeMessage(HOST, { command: "add", manifest: item.manifest, referer: tab && tab.url, ...options });
        } catch {
          answer = { ok: false, error: `${NAME} isn't set up for this browser, or isn't installed.` };
        }
      } else {
        answer = await send(item.url, tab && tab.url, options);
      }
      reply(answer || { ok: false, error: `${NAME} didn't answer.` });
    }
  })();
  return true; // answers later
});
