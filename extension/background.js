// Herb's Download Manager for Chrome, Edge, Brave, Vivaldi and Firefox (SPECIFICATION.md §13): hands each new
// download to the app through native messaging (the app's `hdm`), and the browser keeps any download the app
// doesn't take: when the app isn't open, the link doesn't work for it, or the window is private. It also notes the
// videos and sounds each tab plays (§15), which the toolbar button's list (popup.js) adds to the app's queue.

// The naming rules (naming.js): loaded first by Firefox from the manifest, here by Chrome's service worker.
if (typeof nameByRule === "undefined" && typeof importScripts === "function") importScripts("naming.js");

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
  // As the browser asks: its User-Agent and the page, which guarded streams want with every piece.
  const message = { command: "add", urls: [url], referer: referer || "", cookies: await cookiesFor(url), check: true, user_agent: navigator.userAgent, ...extra };
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

// Right-click a link: download it now with Herb's Download Manager, or add it to the queue.
async function makeMenu() {
  await api.contextMenus.removeAll();
  const contexts = ["link", "image", "video", "audio"];
  api.contextMenus.create({ id: "hdm-link", title: `Download Now with ${NAME}`, contexts });
  api.contextMenus.create({ id: "hdm-link-queue", title: `Add to Queue in ${NAME}`, contexts });
}
api.runtime.onInstalled.addListener(() => { makeMenu(); showState(); applyPanel(); });
api.runtime.onStartup.addListener(() => { makeMenu(); showState(); applyPanel(); });

api.contextMenus.onClicked.addListener(async (info) => {
  // A listed item's Download Now (…-now) or Add to Queue (…-queue).
  const pick = /^hdm-media-(\d+)-(now|queue)$/.exec(String(info.menuItemId));
  if (pick) {
    const chosen = menuItems[Number(pick[1])];
    if (!chosen) return;
    // The site's rule says what choosing it does; "ask" has no field here, so it adds under the rule's name.
    const rule = ruleFor(await rulesSetting(), chosen.tab.url);
    const action = (rule && rule.action) || "download";
    if (action === "copy" && chosen.item.url) {
      await api.scripting.executeScript({ target: { tabId: chosen.tab.id }, func: (t) => navigator.clipboard.writeText(t), args: [chosen.item.url] }).catch(() => {});
      return;
    }
    const answer = await addItem(chosen.tab, chosen.item, { soundOnly: action === "sound", start: pick[2] === "now" });
    if (!(answer && answer.ok)) await tell((answer && answer.error) || `${NAME} didn't answer.`);
    return;
  }
  if (info.menuItemId !== "hdm-link" && info.menuItemId !== "hdm-link-queue") return;
  const url = info.linkUrl || info.srcUrl;
  if (!web(url)) return;
  const answer = await send(url, info.pageUrl, info.menuItemId === "hdm-link-queue" ? { queue: true } : {});
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
  await refreshMenu().catch(() => {});
}

// The button's number: what the tab has to download (or "off" while the hand-over is off).
async function countFor(tabId, items) {
  if (!(await isOn())) return;
  items = items || (await itemsOf(tabId));
  const n = items.filter((i) => !i.hidden && !i.removed).length;
  await api.action.setBadgeText({ tabId, text: n ? String(n) : "" }).catch(() => {});
}

// What a response is: {kind: "stream" | "dash" | "video" | "audio", size}, or null.
// A file's name from its Content-Disposition (`filename*=UTF-8''…` before `filename="…"`), or "".
function dispositionName(header) {
  if (!header) return "";
  const star = /filename\*\s*=\s*([^']*)'[^']*'([^;]+)/i.exec(header);
  if (star) {
    try {
      return decodeURIComponent(star[2].trim().replace(/^"|"$/g, ""));
    } catch {
      // a broken encoding: the plain name below
    }
  }
  const plain = /filename\s*=\s*("([^"]*)"|[^;]+)/i.exec(header);
  return plain ? (plain[2] ?? plain[1]).trim() : "";
}

// An address that names a stream without saying so: a playlist API (`/api/playlist/master/`), `hls` as a part of
// its path, or `m3u8` in its query. Read once to see whether it is one (at most 20 a page).
const MAYBE_STREAM = (url) => {
  const [path, query = ""] = url.split(/[?#]/);
  return /\/api\/playlist\/master\//i.test(path) || /(^|[/_.-])hls([/_.-]|$)/i.test(new URL(path).pathname) || /m3u8/i.test(query);
};

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
  const named = dispositionName(headerOf(d, "content-disposition"));
  if (!typed && !untyped && MAYBE_STREAM(d.url) && !/^(image|font)\//.test(mime) && d.type !== "media") return { kind: "stream", size: 0, maybe: true };
  if (!typed && untyped && MAYBE_STREAM(d.url) && !MEDIA_EXT.test(path) && !MEDIA_EXT.test(named)) return { kind: "stream", size: 0, maybe: true };
  if (!typed && !((MEDIA_EXT.test(path) || MEDIA_EXT.test(named)) && untyped)) return null;
  const range = headerOf(d, "content-range");
  const total = /\/\s*(\d+)\s*$/.exec(range);
  const size = total ? Number(total[1]) : d.statusCode === 206 ? 0 : Number(headerOf(d, "content-length")) || 0;
  // A <video>'s or <audio>'s own file comes in ranges; a script's fetch of a range or a small file is a piece.
  if (d.type !== "media" && (d.statusCode === 206 || range || size < FETCHED_MIN)) return null;
  if (d.type === "media" && size > 0 && size < SOUND_MIN) return null;
  const audio = mime.startsWith("audio/") || (!mime.startsWith("video/") && /\.(mp3|m4a|ogg|oga|opus|flac|wav)$/i.test(named || path));
  return { kind: audio ? "audio" : "video", size, mime, name: named };
}

// A tag's attribute, quoted or not: RESOLUTION=1920x1080, LANGUAGE="de".
const attrOf = (line, name) => (new RegExp(`[:,]${name}=("([^"]*)"|[^,]*)`).exec(line) || [])[2] ?? (new RegExp(`[:,]${name}=([^,]*)`).exec(line) || [])[1];

// A playlist's first bytes, fetched again with the site's cookies: a master playlist's quality and sound playlists
// (`urls`, to hide them from the list), its qualities (the most bits a second for each picture size, largest first,
// HDR left out when a standard version exists) and its sound languages. `null` when it isn't a playlist.
async function variantsOf(url) {
  try {
    const r = await fetchAsPage(url);
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
          // H.264 or H.265 (every player opens them) is preferred to VP9 or AV1 at the same size, as the app does.
          plays: !/vp0?[89]|av01/i.test(attrOf(l, "CODECS") || ""),
        });
      }
      const uri = /^#EXT-X-MEDIA:.*URI="([^"]+)"/.exec(l);
      if (uri) urls.push(new URL(uri[1], url).href);
      const language = l.startsWith("#EXT-X-MEDIA:") && attrOf(l, "TYPE") === "AUDIO" && uri && attrOf(l, "LANGUAGE");
      if (language && !sounds.some((s) => s.language === language)) sounds.push({ language, name: attrOf(l, "NAME") || language });
    });
    const standard = variants.filter((v) => !v.hdr);
    const qualities = [];
    for (const v of (standard.length ? standard : variants).filter((v) => v.lines).sort((a, b) => b.lines - a.lines || b.plays - a.plays || b.bandwidth - a.bandwidth)) {
      if (!qualities.some((q) => q.lines === v.lines)) qualities.push({ lines: v.lines, bandwidth: v.bandwidth });
    }
    // Live: a media playlist with no end (a master's is its first quality's), read with a 5-second limit.
    const ended = (t) => /#EXT-X-ENDLIST|#EXT-X-PLAYLIST-TYPE:VOD/.test(t);
    let live = !variants.length && !ended(text);
    if (variants.length) {
      try {
        const first = await fetchAsPage(urls[0], AbortSignal.timeout(5000));
        live = !ended((await first.text()).slice(0, 256 * 1024));
      } catch {
        live = false;
      }
    }
    return { urls, qualities, sounds, live };
  } catch {
    return { urls: [], qualities: [], sounds: [], live: false };
  }
}

// The headers the page sent for a stream's playlist, kept 10 minutes: a playlist fetched again is asked for as the
// page asked (its tokens, its Referer and Origin), or a site that checks them answers with an error.
const pageHeaders = new Map();
const SKIP_HEADERS = /^(cookie|host|content-length|connection|accept-encoding|sec-|priority|upgrade-insecure-requests|range)/i;
const FORBIDDEN = /^(referer|origin|user-agent)$/i;
if (api.webRequest && api.webRequest.onSendHeaders) {
  const spec = ["requestHeaders"];
  // Chrome hides Referer and Origin from extensions unless asked with "extraHeaders"; Firefox has no such option.
  if (api.webRequest.OnSendHeadersOptions && api.webRequest.OnSendHeadersOptions.EXTRA_HEADERS) spec.push("extraHeaders");
  api.webRequest.onSendHeaders.addListener(
    (d) => {
      if (d.tabId < 0 || !web(d.url)) return;
      const path = pathOf(d.url).toLowerCase();
      if (!/\.(m3u8|mpd)$/.test(path) && !MAYBE_STREAM(d.url)) return;
      const now = Date.now();
      for (const [k, v] of pageHeaders) if (now - v.at > 10 * 60 * 1000) pageHeaders.delete(k);
      pageHeaders.set(d.url, { at: now, headers: (d.requestHeaders || []).filter((h) => !SKIP_HEADERS.test(h.name)) });
    },
    { urls: ["<all_urls>"], types: ["xmlhttprequest", "media", "other"] },
    spec,
  );
}

// Fetches a playlist with the site's cookies and the headers its page sent for it. Headers a script may set go with
// the request; Referer, Origin and User-Agent, which it may not, are set by a browser rule for this one address,
// taken away again after.
let ruleId = 9000;
async function fetchAsPage(url, signal) {
  const kept = pageHeaders.get(url);
  const headers = {};
  const forced = [];
  for (const h of (kept && kept.headers) || []) {
    if (FORBIDDEN.test(h.name)) forced.push({ header: h.name, operation: "set", value: h.value });
    else headers[h.name] = h.value;
  }
  const dnr = api.declarativeNetRequest;
  const id = (ruleId = ruleId >= 9999 ? 9000 : ruleId + 1);
  if (forced.length && dnr && dnr.updateSessionRules) {
    await dnr.updateSessionRules({
      removeRuleIds: [id],
      addRules: [{ id, priority: 1, action: { type: "modifyHeaders", requestHeaders: forced },
                   condition: { urlFilter: `|${url}|`, resourceTypes: ["xmlhttprequest"] } }],
    }).catch(() => {});
  }
  try {
    return await fetch(url, { credentials: "include", headers, signal });
  } finally {
    if (forced.length && dnr && dnr.updateSessionRules) await dnr.updateSessionRules({ removeRuleIds: [id] }).catch(() => {});
  }
}

// ---- Refresh: what the page plays now (§15) --------------------------------------------------------------------
//
// Sites that change videos as you scroll (TikTok, reels) fetch the next ones before they're shown, and play them
// through a blob: address, so the list fills with videos that aren't on screen. Refresh asks the page's players
// what they play: a file's own address, or for a blob: player its length and picture size, matched against the
// files the tab fetched (their length read from their index, the first bytes). The one playing goes first, marked
// "Playing now", and files no player on the page holds any more leave the list; with no match nothing is lost.

// Runs in the page: each video and sound element, what it plays and whether it's playing and on screen.
function playersOnPage() {
  return [...document.querySelectorAll("video, audio")].map((m) => {
    const r = m.getBoundingClientRect();
    const seen = Math.max(0, Math.min(r.right, innerWidth) - Math.max(r.left, 0)) * Math.max(0, Math.min(r.bottom, innerHeight) - Math.max(r.top, 0));
    return {
      src: m.currentSrc || m.src || "",
      playing: !m.paused && !m.ended && m.readyState > 2,
      seconds: Number.isFinite(m.duration) ? m.duration : 0,
      width: m.videoWidth || 0,
      height: m.videoHeight || 0,
      seen,
    };
  }).filter((p) => p.src || p.seconds > 0);
}

// Fetches `url` as its page would (the site's cookies and the page as Referer, set by a browser rule for this one
// address, as a server like TikTok's refuses a file to anyone without its cookie).
async function fetchForPage(url, page, init = {}) {
  const forced = [];
  const cookies = await cookiesFor(url);
  if (cookies) forced.push({ header: "cookie", operation: "set", value: cookies });
  if (page) forced.push({ header: "referer", operation: "set", value: page });
  const dnr = api.declarativeNetRequest;
  const id = (ruleId = ruleId >= 9999 ? 9000 : ruleId + 1);
  if (forced.length && dnr && dnr.updateSessionRules) {
    await dnr.updateSessionRules({
      removeRuleIds: [id],
      addRules: [{ id, priority: 1, action: { type: "modifyHeaders", requestHeaders: forced },
                   condition: { urlFilter: `|${url}|`, resourceTypes: ["xmlhttprequest"] } }],
    }).catch(() => {});
  }
  try {
    return await fetch(url, init);
  } finally {
    if (forced.length && dnr && dnr.updateSessionRules) await dnr.updateSessionRules({ removeRuleIds: [id] }).catch(() => {});
  }
}

// A preview in the list (a <video> in the extension's own page) asks for the file as its page would, for two
// minutes: the rule stays while the browser reads the picture and length.
let mediaRuleId = 10000;
async function mediaAccess(url, page) {
  const dnr = api.declarativeNetRequest;
  if (!web(url) || !dnr || !dnr.updateSessionRules) return;
  const forced = [];
  const cookies = await cookiesFor(url);
  if (cookies) forced.push({ header: "cookie", operation: "set", value: cookies });
  if (page && web(page)) forced.push({ header: "referer", operation: "set", value: page });
  if (!forced.length) return;
  const id = (mediaRuleId = mediaRuleId >= 10099 ? 10000 : mediaRuleId + 1);
  await dnr.updateSessionRules({
    removeRuleIds: [id],
    addRules: [{ id, priority: 1, action: { type: "modifyHeaders", requestHeaders: forced },
                 condition: { urlFilter: `|${url}|`, resourceTypes: ["media"] } }],
  }).catch(() => {});
  const timer = setTimeout(() => dnr.updateSessionRules({ removeRuleIds: [id] }).catch(() => {}), 120000);
  // (In the tests' Node, the timer mustn't keep the run alive.)
  if (timer && timer.unref) timer.unref();
}

// An MP4 file's length and picture size, from its index when it comes first (as a player wants it): null when the
// first 512 KB don't hold it.
async function fileFacts(url, page) {
  let bytes;
  try {
    const r = await fetchForPage(url, page, { headers: { Range: "bytes=0-524287" }, signal: AbortSignal.timeout(8000) });
    if (!r.ok) return null;
    bytes = new Uint8Array(await r.arrayBuffer());
  } catch {
    return null;
  }
  return mp4Facts(bytes);
}

function mp4Facts(b) {
  const u32 = (i) => ((b[i] << 24) >>> 0) + (b[i + 1] << 16) + (b[i + 2] << 8) + b[i + 3];
  const type = (i) => String.fromCharCode(b[i + 4], b[i + 5], b[i + 6], b[i + 7]);
  // The boxes from `from` to `to`: [type, start of contents, end].
  const boxes = (from, to) => {
    const out = [];
    for (let i = from; i + 8 <= to;) {
      let size = u32(i);
      let head = 8;
      if (size === 1) {
        size = u32(i + 8) * 2 ** 32 + u32(i + 12);
        head = 16;
      } else if (size === 0) size = to - i;
      if (size < head) break;
      out.push([type(i), i + head, Math.min(i + size, to)]);
      i += size;
    }
    return out;
  };
  const moov = boxes(0, b.length).find(([t, , end]) => t === "moov" && end <= b.length);
  if (!moov) return null;
  const inside = boxes(moov[1], moov[2]);
  const mvhd = inside.find(([t]) => t === "mvhd");
  if (!mvhd) return null;
  const at = mvhd[1];
  const v1 = b[at] === 1;
  const scale = u32(at + (v1 ? 20 : 12));
  const duration = v1 ? u32(at + 24) * 2 ** 32 + u32(at + 28) : u32(at + 16);
  let width = 0;
  let height = 0;
  for (const [t, start, end] of inside) {
    if (t !== "trak") continue;
    const tkhd = boxes(start, end).find(([k]) => k === "tkhd");
    if (!tkhd) continue;
    // Its last two fields: the picture's width and height (16.16 fixed point); a sound track's are 0.
    const w = u32(tkhd[2] - 8) >>> 16;
    const h = u32(tkhd[2] - 4) >>> 16;
    if (w && h) {
      width = w;
      height = h;
      break;
    }
  }
  return scale ? { seconds: duration / scale, width, height } : null;
}

// Files' lengths and picture sizes (`mp4Facts`), by address; null when unknown.
const factsOf = new Map();

async function refresh(tabId) {
  const tab = await api.tabs.get(tabId).catch(() => null);
  if (!tab) return { ok: false };
  let players = [];
  try {
    const results = await api.scripting.executeScript({ target: { tabId, allFrames: true }, func: playersOnPage });
    players = results.flatMap((r) => (r && r.result) || []);
  } catch {
    return { ok: false, error: "This page can't be read." };
  }
  const page = web(tab.url) ? tab.url : "";
  // Each file's length and size, read once (in parallel) and kept.
  const unknown = (await itemsOf(tabId)).filter((i) => (i.kind === "video" || i.kind === "audio") && i.url && !factsOf.has(i.url));
  await Promise.all(unknown.map(async (f) => factsOf.set(f.url, await fileFacts(f.url, page))));
  if (factsOf.size > 200) factsOf.delete(factsOf.keys().next().value);
  let playing = null;
  // Written in turn with what the tab fetches meanwhile.
  await (noting = noting.then(async () => {
    const items = await itemsOf(tabId);
    // A playing file the list doesn't hold yet (fetched before the list started looking): added.
    for (const p of players) {
      if (web(p.src) && !items.some((i) => i.url === p.src)) {
        items.push({ url: p.src, kind: p.width ? "video" : "audio", size: 0, seen: Date.now(), hidden: false });
      }
    }
    const files = items.filter((i) => (i.kind === "video" || i.kind === "audio") && i.url && !i.hidden && !i.removed);
    const close = (a, b) => a > 0 && b > 0 && Math.abs(a - b) < 0.5;
    // The file a player plays: its own address, else the one as long and as big.
    const fileOf = (p) => {
      if (web(p.src)) return files.find((f) => f.url === p.src) || null;
      const same = files.filter((f) => {
        const facts = factsOf.get(f.url);
        return facts && close(facts.seconds, p.seconds)
          && (!p.width || !facts.width || (facts.width === p.width && facts.height === p.height));
      });
      // The closest in length (a player's own length runs a little over its file's: 10.20 s for 10.08), then the newest.
      const off = (f) => Math.abs(factsOf.get(f.url).seconds - p.seconds);
      return same.sort((a, b) => off(a) - off(b) || (b.seen || 0) - (a.seen || 0))[0] || null;
    };
    // The one playing (the biggest on screen), else the biggest on screen.
    const ranked = [...players].sort((a, b) => Number(b.playing) - Number(a.playing) || b.seen - a.seen);
    const held = new Set();
    for (const p of ranked) {
      const f = fileOf(p);
      if (!f) continue;
      held.add(f);
      if (!playing && (p.playing || p.seen > 0)) playing = f;
    }
    for (const i of items) i.playing = i === playing;
    // Files no player holds any more go, once the page's players were matched at all.
    const kept = held.size ? items.filter((i) => !(i.kind === "video" || i.kind === "audio") || held.has(i)) : items;
    await setItems(tabId, kept);
  }).catch(() => {}));
  return { ok: true, playing: !!playing, players: players.length };
}

// How many addresses each tab had read to see whether they're streams (MAYBE_STREAM), at most 20.
const probes = new Map();

// A DASH page's own pieces: on a page that plays a DASH stream, files under 20 MB are left out.
const DASH_PIECE = 20 << 20;
const pieceOfDash = (i) => (i.kind === "video" || i.kind === "audio") && i.size > 0 && i.size < DASH_PIECE;

let noting = Promise.resolve();
function note(d, found) {
  // One at a time: two answers at once would both read the old list.
  noting = noting.then(async () => {
    const items = await itemsOf(d.tabId);
    if (items.some((i) => i.url === d.url)) return;
    if (found.maybe) {
      const n = probes.get(d.tabId) || 0;
      if (n >= 20) return;
      probes.set(d.tabId, n + 1);
    }
    const item = { url: d.url, kind: found.kind, size: found.size || 0, mime: found.mime || "", hidden: false, seen: Date.now() };
    if (found.name) item.name = found.name;
    const dashPage = found.kind === "dash" || items.some((i) => i.kind === "dash");
    if (dashPage && pieceOfDash(item)) item.hidden = true;
    if (found.kind === "dash") for (const i of items) if (pieceOfDash(i)) i.hidden = true;
    if (found.kind === "stream") {
      const found = await variantsOf(d.url);
      if (found === null) return; // not a playlist after all
      const variants = found.urls;
      // A quality playlist of a master already listed is part of it; a master hides the ones seen before it.
      if (items.some((i) => (i.variants || []).includes(d.url))) item.hidden = true;
      item.variants = variants;
      item.qualities = found.qualities;
      item.sounds = found.sounds;
      item.live = found.live;
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
api.tabs.onRemoved.addListener((id) => {
  probes.delete(id);
  api.storage.session.remove([`media-${id}`, `removed-${id}`]).catch(() => {});
});
// Another page in the tab: what the last one played goes.
api.tabs.onUpdated.addListener((id, change) => {
  if (change.url) {
    probes.delete(id);
    noting = noting.then(() => setItems(id, [])).catch(() => {});
    api.storage.session.remove(`removed-${id}`).catch(() => {});
  }
});
api.tabs.onActivated.addListener(({ tabId }) => {
  countFor(tabId).catch(() => {});
  refreshMenu().catch(() => {});
});

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
      const tab = await api.tabs.get(message.tabId).catch(() => null);
      const order = await orderSetting();
      const rule = tab ? ruleFor(await rulesSetting(), tab.url) : null;
      reply({ on: await isOn(), items: await listed(message.tabId, tab, order), order, panel: await panelSetting(), panelOffered: !!api.sidePanel,
              action: (rule && rule.action) || "download", site: tab && web(tab.url) ? new URL(tab.url).hostname.replace(/^www\./, "") : "" });
    } else if (message.type === "name") {
      // The name an item would get, for "Ask for a name first".
      const tab = await api.tabs.get(message.tabId).catch(() => null);
      const rule = tab ? ruleFor(await rulesSetting(), tab.url) : null;
      const item = message.item;
      const last = decodeURIComponent(pathOf(item.url || "").split("/").pop() || "");
      reply({ name: rule ? await ruleName(rule, item, tab) : item.name || nameFor(item, tab && tab.title) || last || "video.mp4" });
    } else if (message.type === "rules") {
      // "Naming Rules…": the page opens on this site's rule.
      if (message.site) await api.storage.local.set({ newRuleSite: message.site });
      await api.runtime.openOptionsPage();
      reply({ ok: true });
    } else if (message.type === "set-order") {
      await api.storage.local.set({ order: message.order });
      await refreshMenu();
      reply({ ok: true });
    } else if (message.type === "set-panel") {
      await api.storage.local.set({ panel: !!message.on });
      await applyPanel();
      reply({ ok: true, panel: await panelSetting() });
    } else if (message.type === "remove") {
      await remove(message.tabId, message.item);
      reply({ ok: true });
    } else if (message.type === "refresh") {
      reply(await refresh(message.tabId));
    } else if (message.type === "media-access") {
      // A file's own preview in the list: the browser asks for it as the page would (§15).
      await mediaAccess(message.url, message.page);
      reply({ ok: true });
    } else if (message.type === "look-again") {
      // The page loads again, so its player fetches its media again and the list fills as it does.
      await setItems(message.tabId, []);
      await api.tabs.reload(message.tabId).catch(() => {});
      reply({ ok: true });
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
        reply(await api.runtime.sendNativeMessage(HOST, { command: "scan", urls: [tab.url], cookies, referer: tab.url, user_agent: navigator.userAgent }));
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
      reply(await addItem(tab, message.item, message));
    }
  })();
  return true; // answers later
});

// Adds a listed item to the app (from the list or the right-click menu): `message` carries start, quality, sound
// and soundOnly.
async function addItem(tab, item, message) {
  // A file the app found on the page comes with its name; what the tab played is named here, by the site's rule when
  // it has one, or as asked in the list.
  const rule = tab ? ruleFor(await rulesSetting(), tab.url) : null;
  // A site's rule wins over the server's own name (Content-Disposition) or the one a page scan found.
  const name = message.name ? cleanName(message.name) : rule ? await ruleName(rule, item, tab) : item.name || nameFor(item, tab && tab.title);
  // Added to the queue; right after opening the app ("Open and Download"), started at once.
  const options = { queue: !message.start, stream: ["stream", "dash", "page-dash"].includes(item.kind) || item.stream === true, name };
  // A stream's quality and sound chosen in the list (S13, S14 otherwise).
  if (message.quality) options.quality = message.quality;
  if (message.sound !== undefined) options.sound = message.sound;
  // Only its sound, saved as it is (.m4a, or .mp3).
  if (message.soundOnly && options.stream) options.sound_only = true;
  // The rule's sub-folder, inside the folder Settings saves to.
  if (rule && rule.folder) options.subfolder = rule.folder;
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
  return answer || { ok: false, error: `${NAME} didn't answer.` };
}

// ---- naming rules (§13) -------------------------------------------------------------------------------------

const rulesSetting = async () => (await api.storage.local.get({ rules: [] })).rules;

// The extension a name gets: a stream's MP4, or the file's own (from its link, else its type).
function extensionFor(item) {
  if (["stream", "dash", "page-dash"].includes(item.kind)) return "mp4";
  const last = decodeURIComponent(pathOf(item.url || "").split("/").pop() || "");
  const own = /\.([a-z0-9]{2,5})$/i.exec(last);
  if (own && MEDIA_EXT.test(last)) return own[1].toLowerCase();
  return /webm/.test(item.mime || "") ? "webm" : item.kind === "audio" ? (/mpeg/.test(item.mime || "") ? "mp3" : "m4a") : "mp4";
}

// The name a site's rule gives an item, the selector's text read from the page when the pattern uses it.
async function ruleName(rule, item, tab) {
  let selected = "";
  if (rule.selector && /%selector/.test(rule.pattern || "")) {
    try {
      const [result] = await api.scripting.executeScript({
        target: { tabId: tab.id },
        func: (selector) => (document.querySelector(selector)?.textContent || "").trim().slice(0, 300),
        args: [rule.selector],
      });
      selected = (result && result.result) || "";
    } catch {
      selected = "";
    }
  }
  return `${nameByRule(rule, { title: tab.title, url: tab.url, selected })}.${extensionFor(item)}`;
}

// ---- the list's order, the right-click menu and the side panel (§13, §15) -------------------------------------

const orderSetting = async () => (await api.storage.local.get({ order: "smart" })).order;
const panelSetting = async () => !!api.sidePanel && (await api.storage.local.get({ panel: false })).panel;

// A site's name without its subdomains: the last two labels of the host (www.youtube.com → youtube.com).
function siteOf(url) {
  try {
    return new URL(url).hostname.split(".").slice(-2).join(".");
  } catch {
    return "";
  }
}

// ---- Removing an item from the list (×) ---------------------------------------------------------------------------
//
// A removed item stays out of the list while the tab shows the page (fetched again, it isn't listed again); a video
// read from the page's own data, which is read afresh each time the list opens, is noted by its manifest.

const manifestKey = (manifest) => `${manifest.length}:${manifest.slice(0, 200)}:${manifest.slice(-200)}`;

async function removedOf(tabId) {
  const key = `removed-${tabId}`;
  return (await api.storage.session.get({ [key]: [] }))[key];
}

async function remove(tabId, item) {
  if (item.kind === "page-dash" && item.manifest) {
    const gone = await removedOf(tabId);
    gone.push(manifestKey(item.manifest));
    await api.storage.session.set({ [`removed-${tabId}`]: gone.slice(-40) });
    return;
  }
  await (noting = noting.then(async () => {
    const items = await itemsOf(tabId);
    for (const i of items) if (i.url === item.url) i.removed = true;
    await setItems(tabId, items);
  }).catch(() => {}));
}

// The tab's items as the list shows them, in the order chosen: Smart puts DASH, then HLS streams, then video files,
// then sounds; a finished stream before a live one; the page's own site before others; else the order found.
// Oldest and Newest go by when each was found.
async function listed(tabId, tab, order, page = true) {
  const items = (await itemsOf(tabId)).filter((i) => !i.hidden && !i.removed);
  const gone = page ? await removedOf(tabId) : [];
  for (const manifest of page ? await pageManifests(tabId) : []) {
    const about = describeManifest(manifest);
    if (!about.protected && !gone.includes(manifestKey(manifest))) items.unshift({ kind: "page-dash", manifest, url: "", size: 0, seen: 0, ...about });
  }
  const found = (i) => i.seen || 0;
  // What a Refresh found playing comes first, whatever the order.
  const first = (sorted) => sorted.sort((a, b) => Number(!!b.playing) - Number(!!a.playing));
  if (order === "oldest") return first(items.sort((a, b) => found(a) - found(b)));
  if (order === "newest") return first(items.sort((a, b) => found(b) - found(a)));
  const site = tab ? siteOf(tab.url) : "";
  const rank = (i) => [
    { "page-dash": 0, dash: 0, stream: 1, video: 2, file: 2, audio: 3 }[i.kind] ?? 2,
    i.live ? 1 : 0,
    i.url && siteOf(i.url) === site ? 0 : i.kind === "page-dash" ? 0 : 1,
    found(i),
  ];
  return first(items.sort((a, b) => {
    const [x, y] = [rank(a), rank(b)];
    for (let k = 0; k < x.length; k++) if (x[k] !== y[k]) return x[k] - y[k];
    return 0;
  }));
}

// The right-click menu's list of the active tab's media, in the list's order (its first 20; videos a page carries in
// its data are read only when the list opens, so they aren't here): each one's Download Now or Add to Queue.
let menuItems = [];
async function refreshMenu() {
  if (!api.contextMenus) return;
  const [tab] = await api.tabs.query({ active: true, lastFocusedWindow: true }).catch(() => []);
  const items = tab ? (await listed(tab.id, tab, await orderSetting(), false)).slice(0, 20) : [];
  menuItems = items.map((item) => ({ tab, item }));
  await new Promise((done) => api.contextMenus.remove("hdm-media", () => { void api.runtime.lastError; done(); }));
  if (!items.length) return;
  const contexts = ["page", "frame", "video", "audio", "selection"];
  api.contextMenus.create({ id: "hdm-media", title: `${NAME}: media on this page`, contexts });
  items.forEach((item, i) => {
    const streamed = ["stream", "dash", "page-dash"].includes(item.kind);
    const name = streamed ? (tab.title || "Video") : decodeURIComponent(pathOf(item.url).split("/").pop() || item.url);
    const kind = { stream: "Video stream", dash: "Video stream", "page-dash": "Video", video: "Video", audio: "Sound" }[item.kind] || "File";
    const quality = item.quality || (item.qualities && item.qualities[0] ? `${item.qualities[0].lines}p` : "");
    api.contextMenus.create({ id: `hdm-media-${i}`, parentId: "hdm-media", title: [name.slice(0, 60), kind, quality].filter(Boolean).join(" · "), contexts });
    api.contextMenus.create({ id: `hdm-media-${i}-now`, parentId: `hdm-media-${i}`, title: "Download Now", contexts });
    api.contextMenus.create({ id: `hdm-media-${i}-queue`, parentId: `hdm-media-${i}`, title: "Add to Queue", contexts });
  });
}

// "Open in the side panel" (Chrome, Edge): the toolbar button opens the list there instead of as a popup.
async function applyPanel() {
  if (!api.sidePanel) return;
  const on = await panelSetting();
  await api.sidePanel.setPanelBehavior({ openPanelOnActionClick: on }).catch(() => {});
  await api.action.setPopup({ popup: on ? "" : "popup.html" }).catch(() => {});
}
