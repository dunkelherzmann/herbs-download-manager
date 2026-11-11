// The extension's background (SPECIFICATION.md §13, §15) run with a stand-in browser: `node --test tests/extension`.

const test = require("node:test");
const assert = require("node:assert/strict");
const { load, headers } = require("./harness.js");

const PAGE = { id: 1, active: true, title: "Song | Band", url: "https://www.example.com/watch" };
const MASTER = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080\nhi.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=900000,RESOLUTION=640x360\nlo.m3u8\n";
const VOD = "#EXTM3U\n#EXTINF:2,\ns0.ts\n#EXT-X-ENDLIST\n";
const LIVE = "#EXTM3U\n#EXTINF:2,\ns0.ts\n";
const plain = (o) => JSON.parse(JSON.stringify(o));

test("what a response is: streams, files, pieces and what's left out", () => {
  const b = load();
  const kind = (url, type, h, statusCode = 200) => b.get("kindOf")({ tabId: 1, url, type, statusCode, responseHeaders: headers(h) });
  assert.equal(kind("https://a.com/v/master.m3u8", "xmlhttprequest", {}).kind, "stream");
  assert.equal(kind("https://a.com/play", "xmlhttprequest", { "content-type": "application/vnd.apple.mpegurl" }).kind, "stream");
  assert.equal(kind("https://a.com/m.mpd", "xmlhttprequest", {}).kind, "dash");
  assert.equal(kind("https://a.com/hls/seg-1.ts", "xmlhttprequest", { "content-type": "video/mp2t" }), null, "a stream's piece");
  assert.equal(kind("https://a.com/v.mp4?bytestart=0&byteend=9", "xmlhttprequest", { "content-type": "video/mp4" }), null, "a byte-range piece");
  assert.equal(kind("https://a.com/v.mp4", "media", { "content-type": "video/mp4" }, 404), null, "an error answer");
  assert.equal(kind("https://a.com/click.mp3", "media", { "content-type": "audio/mpeg", "content-length": "1000" }), null, "a short sound");
  assert.equal(kind("https://a.com/small.mp4", "xmlhttprequest", { "content-type": "video/mp4", "content-length": "5000" }), null, "a script's small fetch");
  const film = plain(kind("https://a.com/film.mp4", "media", { "content-type": "video/mp4", "content-length": "90000000" }));
  assert.deepEqual(film, { kind: "video", size: 90000000, mime: "video/mp4", name: "" });
});

test("a file's name comes from its Content-Disposition, and that name can show it's media", () => {
  const b = load();
  const name = b.get("dispositionName");
  assert.equal(name("attachment; filename*=UTF-8''Caf%C3%A9%20Live.mp4; filename=\"x.mp4\""), "Café Live.mp4");
  assert.equal(name("inline; filename=\"Show 1.mp4\""), "Show 1.mp4");
  assert.equal(name(""), "");
  const d = { tabId: 1, url: "https://a.com/dl?id=7", type: "xmlhttprequest", statusCode: 200,
              responseHeaders: headers({ "content-type": "application/octet-stream", "content-length": "5000000", "content-disposition": "attachment; filename=\"Talk.mp3\"" }) };
  assert.deepEqual(plain(b.get("kindOf")(d)), { kind: "audio", size: 5000000, mime: "application/octet-stream", name: "Talk.mp3" });
});

test("an address that hints at a stream is read, and kept only when it is one", async () => {
  const b = load({ tabs: [PAGE], fetch: (url) => (url.endsWith("/master/1") ? { body: VOD } : { body: "{\"not\": \"a playlist\"}" }) });
  const kind = (url, h = {}) => b.get("kindOf")({ tabId: 1, url, type: "xmlhttprequest", statusCode: 200, responseHeaders: headers(h) });
  assert.equal(kind("https://a.com/api/playlist/master/1", { "content-type": "application/json" }).maybe, true);
  assert.equal(kind("https://a.com/hls/video/abc", { "content-type": "text/plain" }).maybe, true);
  assert.equal(kind("https://a.com/page?format=m3u8").maybe, true);
  assert.equal(kind("https://a.com/hls/logo.png", { "content-type": "image/png" }), null);
  assert.equal(kind("https://a.com/x/data.json", { "content-type": "application/json" }), null);
  await b.respond({ url: "https://a.com/api/playlist/master/1", responseHeaders: headers({ "content-type": "application/json" }) });
  await b.respond({ url: "https://a.com/api/playlist/master/2", responseHeaders: headers({ "content-type": "application/json" }) });
  assert.deepEqual((await b.items()).map((i) => i.url), ["https://a.com/api/playlist/master/1"], "the one that wasn't a playlist is dropped");
});

test("at most 20 hinted addresses are read on a page", async () => {
  const b = load({ tabs: [PAGE], fetch: () => ({ body: "{}" }) });
  for (let n = 0; n < 30; n++) await b.respond({ url: `https://a.com/hls/thing/${n}`, responseHeaders: headers({ "content-type": "text/plain" }) });
  assert.equal(b.fetches.length, 20);
});

test("a master playlist hides its qualities, and says whether it's live", async () => {
  const b = load({ tabs: [PAGE], fetch: (url) => ({ body: url.endsWith("master.m3u8") ? MASTER : url.includes("live") ? LIVE : VOD }) });
  await b.respond({ url: "https://cdn.example.com/v/hi.m3u8" });
  await b.respond({ url: "https://cdn.example.com/v/master.m3u8" });
  const items = await b.items();
  assert.deepEqual(items.map((i) => [i.url.split("/").pop(), i.hidden]), [["hi.m3u8", true], ["master.m3u8", false]]);
  const master = items[1];
  assert.deepEqual(plain(master.qualities), [{ lines: 1080, bandwidth: 5000000 }, { lines: 360, bandwidth: 900000 }]);
  assert.equal(master.live, false);
  await b.respond({ url: "https://cdn.example.com/live/index.m3u8" });
  assert.equal((await b.items()).pop().live, true, "a media playlist with no end");
});

test("on a page that plays DASH, files under 20 MB are its pieces", async () => {
  const b = load({ tabs: [PAGE] });
  const file = (url, size) => ({ url, type: "media", responseHeaders: headers({ "content-type": "video/mp4", "content-length": String(size) }) });
  await b.respond(file("https://cdn.example.com/before.mp4", 5_000_000));
  await b.respond({ url: "https://cdn.example.com/m.mpd" });
  await b.respond(file("https://cdn.example.com/after.mp4", 3_000_000));
  await b.respond(file("https://cdn.example.com/whole.mp4", 300_000_000));
  const shown = (await b.items()).filter((i) => !i.hidden).map((i) => i.url.split("/").pop());
  assert.deepEqual(shown, ["m.mpd", "whole.mp4"]);
});

test("the list's order: Smart, Oldest and Newest", async () => {
  const b = load({ tabs: [PAGE] });
  const at = (url, kind, seen, extra = {}) => ({ url, kind, seen, hidden: false, size: 0, ...extra });
  await b.chrome.storage.session.set({ "media-1": [
    at("https://cdn.other.net/ad.mp4", "video", 1),
    at("https://video.example.com/live/master.m3u8", "stream", 2, { live: true }),
    at("https://www.example.com/song.mp3", "audio", 3),
    at("https://cdn.other.net/x/master.m3u8", "stream", 4),
    at("https://video.example.com/vod/master.m3u8", "stream", 5),
    at("https://video.example.com/a.mpd", "dash", 6),
    at("https://www.example.com/clip.mp4", "video", 7),
  ] });
  const order = async (o) => (await b.get("listed")(1, PAGE, o, false)).map((i) => i.seen).join(" ");
  assert.equal(await order("smart"), "6 5 4 2 7 1 3", "DASH, streams (finished, own site first; then live), files (own site first), sounds");
  assert.equal(await order("oldest"), "1 2 3 4 5 6 7");
  assert.equal(await order("newest"), "7 6 5 4 3 2 1");
});

test("the right-click menu lists the active tab's media, each to add to the queue or download now", async () => {
  const b = load({ tabs: [PAGE] });
  await b.chrome.storage.session.set({ "media-1": [{ url: "https://cdn.example.com/v/master.m3u8", kind: "stream", seen: 1, hidden: false, size: 0, qualities: [{ lines: 720 }] }] });
  await b.get("refreshMenu")();
  assert.equal(b.menus.get("hdm-media-0").title, "Song | Band · Video stream · 720p");
  assert.equal(b.menus.get("hdm-media-0-queue").title, "Add to Queue");
  assert.equal(b.menus.get("hdm-media-0-now").title, "Download Now");
  b.chrome.contextMenus.onClicked.fire({ menuItemId: "hdm-media-0-queue" }, PAGE);
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(b.sent.length, 1);
  assert.deepEqual([b.sent[0].urls[0], b.sent[0].stream, b.sent[0].name, b.sent[0].queue], ["https://cdn.example.com/v/master.m3u8", true, "Song | Band.mp4", true]);
  b.chrome.contextMenus.onClicked.fire({ menuItemId: "hdm-media-0-now" }, PAGE);
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(b.sent[1].queue, false, "Download Now starts it at once");
  // A link: Download Now, or Add to Queue.
  b.chrome.contextMenus.onClicked.fire({ menuItemId: "hdm-link-queue", linkUrl: "https://www.example.com/a.zip", pageUrl: PAGE.url }, PAGE);
  b.chrome.contextMenus.onClicked.fire({ menuItemId: "hdm-link", linkUrl: "https://www.example.com/b.zip", pageUrl: PAGE.url }, PAGE);
  await new Promise((r) => setTimeout(r, 20));
  assert.deepEqual(b.sent.slice(2).map((m) => [m.urls[0], m.queue === true]), [["https://www.example.com/a.zip", true], ["https://www.example.com/b.zip", false]]);  // The menu asks for the app: hdm opens it in the background when it isn't running.
  assert.deepEqual(b.sent.map((m) => m.open), [true, true, true, true]);
});

test("a site's naming rule names what's added, gives its sub-folder, and says what a click does", async () => {
  const b = load({ tabs: [PAGE], script: (selector) => `Text of ${selector}` });
  await b.chrome.storage.local.set({ rules: [{ site: "example.com", pattern: "%site - %selector - %title", selector: "h1", replace: [[" | ", " by "]], max: 0, folder: "Music", action: "sound" }] });
  const state = await b.ask({ type: "state", tabId: 1 });
  assert.equal(state.action, "sound");
  assert.equal(state.site, "example.com");
  const item = { url: "https://cdn.example.com/v/master.m3u8", kind: "stream" };
  assert.equal((await b.ask({ type: "name", tabId: 1, item })).name, "example - Text of h1 - Song by Band.mp4");
  await b.ask({ type: "add", tabId: 1, item, soundOnly: true });
  assert.deepEqual([b.sent[0].name, b.sent[0].subfolder, b.sent[0].sound_only], ["example - Text of h1 - Song by Band.mp4", "Music", true]);
  // A name typed in the list wins.
  await b.ask({ type: "add", tabId: 1, item, name: "Mine: 1/2.mp4" });
  assert.equal(b.sent[1].name, "Mine 1 2.mp4");
});

test("without a rule, a stream is named after its page and a file keeps its own name", async () => {
  const b = load({ tabs: [PAGE] });
  await b.ask({ type: "add", tabId: 1, item: { url: "https://cdn.example.com/v/master.m3u8", kind: "stream" }, quality: "720", sound: "de" });
  await b.ask({ type: "add", tabId: 1, item: { url: "https://cdn.example.com/talk.mp3", kind: "audio" } });
  await b.ask({ type: "add", tabId: 1, item: { url: "https://cdn.example.com/dl?id=1", kind: "audio", name: "Talk.mp3" } });
  assert.deepEqual(b.sent.map((m) => [m.name, m.subfolder ?? null]), [["Song | Band.mp4", null], [null, null], ["Talk.mp3", null]]);
  assert.deepEqual([b.sent[0].quality, b.sent[0].sound], ["720", "de"]);
});

test("a playlist is read again with the headers its page sent", async () => {
  const b = load({ tabs: [PAGE], fetch: () => ({ body: VOD }) });
  const url = "https://cdn.example.com/v/index.m3u8";
  b.chrome.webRequest.onSendHeaders.fire({ tabId: 1, url, requestHeaders: headers({
    Referer: "https://www.example.com/watch", Origin: "https://www.example.com", "X-Token": "abc", Cookie: "s=1", Range: "bytes=0-" }) });
  await b.respond({ url });
  const read = b.fetches.find((f) => f.url === url);
  assert.deepEqual(plain(read.options.headers), { "X-Token": "abc" }, "headers a script may set; cookies go with credentials");
  assert.equal(read.options.credentials, "include");
  const rule = b.rules.added.find((r) => r.condition.urlFilter === `|${url}|`);
  assert.deepEqual(plain(rule.action.requestHeaders).map((h) => h.header).sort(), ["Origin", "Referer"]);
  assert.ok(b.rules.removed.includes(rule.id), "the rule goes again");
});

test("a stream a player in an embedded frame fetched is handed over with the player's own Referer", async () => {
  const b = load({ tabs: [PAGE], fetch: () => ({ body: VOD }) });
  const url = "https://cdn.example.net/hls/abc/index.m3u8";
  b.chrome.webRequest.onSendHeaders.fire({ tabId: 1, url, type: "xmlhttprequest", requestHeaders: headers({
    Referer: "https://player.example.org/e/42", Origin: "https://player.example.org", "User-Agent": "Player UA" }) });
  await b.ask({ type: "add", tabId: 1, item: { url, kind: "stream" } });
  const sent = plain(b.sent.find((m) => m.command === "add" && m.urls[0] === url));
  assert.equal(sent.referer, "https://player.example.org/e/42", "the player's frame, not the tab's page");
  assert.equal(sent.user_agent, "Player UA");
  // A stream from a host the tab never asked anything of goes with the tab's page, as before.
  const other = "https://elsewhere.example.com/hls/other/index.m3u8";
  await b.ask({ type: "add", tabId: 1, item: { url: other, kind: "stream" } });
  assert.equal(plain(b.sent.find((m) => m.urls && m.urls[0] === other)).referer, PAGE.url);
});

test("a player that sent no Referer is handed over without one, even after the background script slept", async () => {
  const b = load({ tabs: [PAGE], fetch: () => ({ body: VOD }) });
  const url = "https://player.example.org/m3u8/7075342";
  b.chrome.webRequest.onSendHeaders.fire({ tabId: 1, url, type: "xmlhttprequest", requestHeaders: headers({ Origin: "https://player.example.org", "User-Agent": "Player UA" }) });
  await b.respond({ url, responseHeaders: headers({ "content-type": "application/vnd.apple.mpegurl" }) });
  // Put to sleep and woken again: what it saw in memory is gone, what it kept with the item isn't.
  b.get("pageHeaders").clear();
  b.get("hostHeaders").clear();
  await b.ask({ type: "add", tabId: 1, item: { url, kind: "stream" } });
  const sent = plain(b.sent.at(-1));
  assert.equal(sent.referer, "", "none, as the player sent none: not the tab's page");
  assert.equal(sent.origin, "https://player.example.org");
  assert.equal(sent.user_agent, "Player UA");
});

test("another page in the tab forgets what the last one played", async () => {
  const b = load({ tabs: [PAGE] });
  await b.respond({ url: "https://cdn.example.com/m.mpd" });
  assert.equal((await b.items()).length, 1);
  b.chrome.tabs.onUpdated.fire(1, { url: "https://www.example.com/next" });
  await b.get("noting");
  assert.equal((await b.items()).length, 0);
});

// ---- Refresh (§15) ----------------------------------------------------------------------------------------------

// A tiny MP4 whose index comes first: `seconds` long, its picture `width` × `height`.
function mp4(seconds, width = 720, height = 1280) {
  const box = (type, ...parts) => {
    const body = Buffer.concat(parts);
    const head = Buffer.alloc(8);
    head.writeUInt32BE(body.length + 8);
    head.write(type, 4, "latin1");
    return Buffer.concat([head, body]);
  };
  const mvhd = Buffer.alloc(100);
  mvhd.writeUInt32BE(1000, 12); // timescale
  mvhd.writeUInt32BE(Math.round(seconds * 1000), 16); // duration
  const tkhd = Buffer.alloc(84);
  tkhd.writeUInt32BE(width << 16, 76);
  tkhd.writeUInt32BE(height << 16, 80);
  const sound = Buffer.alloc(84); // a sound track: no picture size
  const moov = box("moov", box("mvhd", mvhd), box("trak", box("tkhd", sound)), box("trak", box("tkhd", tkhd)));
  return new Uint8Array(Buffer.concat([box("ftyp", Buffer.from("isom0000")), moov, box("mdat", Buffer.alloc(16))]));
}

const TIKTOK = { id: 1, active: true, title: "Funny dog | TikTok", url: "https://www.tiktok.com/foryou" };
const clip = (n) => `https://v16-webapp-prime.tiktok.com/video/tos/${n}/?mime_type=video_mp4&tk=tt_chain_token`;

test("an MP4's length and picture size are read from its index", () => {
  const b = load();
  assert.deepEqual(plain(b.get("mp4Facts")(mp4(12.5))), { seconds: 12.5, width: 720, height: 1280 });
  assert.equal(b.get("mp4Facts")(new Uint8Array(64)), null, "no index: unknown");
});

test("Refresh puts the video playing now first and drops what the page no longer holds", async () => {
  const lengths = { a: 10, b: 12.5, c: 30 };
  const b = load({
    tabs: [TIKTOK],
    cookies: [{ name: "tt_chain_token", value: "secret" }],
    fetch: (url) => ({ status: 206, body: mp4(lengths[url.split("/")[5]]) }),
    // A blob: player playing the 12.5-second video, and the next one waiting below it.
    script: () => [
      { src: "blob:https://www.tiktok.com/1", playing: true, seconds: 12.52, width: 720, height: 1280, seen: 300000 },
      { src: "blob:https://www.tiktok.com/2", playing: false, seconds: 30, width: 720, height: 1280, seen: 0 },
    ],
  });
  await b.chrome.storage.session.set({ "media-1": ["a", "b", "c"].map((n, i) => ({ url: clip(n), kind: "video", size: 33e6, seen: i + 1, hidden: false })) });
  const answer = await b.ask({ type: "refresh", tabId: 1 });
  assert.deepEqual(plain(answer), { ok: true, playing: true, players: 2 });
  const list = await b.get("listed")(1, TIKTOK, "smart", false);
  assert.deepEqual(list.map((i) => [i.url.split("/")[5], !!i.playing]), [["b", true], ["c", false]], "the 10-second one is no longer on the page");
  // Each file's start was read as its page would ask: the site's cookie and the page as Referer.
  const rule = b.rules.added.find((r) => r.condition.urlFilter === `|${clip("a")}|`);
  assert.deepEqual(plain(rule.action.requestHeaders), [
    { header: "cookie", operation: "set", value: "tt_chain_token=secret" },
    { header: "referer", operation: "set", value: TIKTOK.url },
  ]);
  assert.equal(b.fetches.find((f) => f.url === clip("a")).options.headers.Range, "bytes=0-524287");
});

test("Refresh keeps the list when nothing on the page matches, and adds a file a player plays by its address", async () => {
  const b = load({
    tabs: [PAGE],
    fetch: () => ({ status: 403, body: "" }),
    script: () => [{ src: "blob:https://www.example.com/1", playing: true, seconds: 8, width: 640, height: 360, seen: 1000 }],
  });
  await b.chrome.storage.session.set({ "media-1": [{ url: "https://cdn.example.com/a.mp4", kind: "video", size: 5e6, seen: 1, hidden: false }] });
  assert.equal((await b.ask({ type: "refresh", tabId: 1 })).playing, false);
  assert.equal((await b.items()).length, 1, "nothing matched: nothing dropped");

  const direct = load({ tabs: [PAGE], script: () => [{ src: "https://cdn.example.com/now.mp4", playing: true, seconds: 8, width: 640, height: 360, seen: 1000 }] });
  assert.equal((await direct.ask({ type: "refresh", tabId: 1 })).playing, true);
  const [now] = await direct.items();
  assert.deepEqual([now.url, now.kind, now.playing], ["https://cdn.example.com/now.mp4", "video", true]);
});

test("a file's preview in the list is asked for with the page's cookies and address", async () => {
  const b = load({ tabs: [TIKTOK], cookies: [{ name: "tt_chain_token", value: "secret" }] });
  await b.ask({ type: "media-access", url: clip("a"), page: TIKTOK.url });
  const [rule] = b.rules.added;
  assert.deepEqual(plain(rule.condition), { urlFilter: `|${clip("a")}|`, resourceTypes: ["media"] });
  assert.deepEqual(plain(rule.action.requestHeaders).map((h) => h.header), ["cookie", "referer"]);
});

// ---- Removing from the list (×) ---------------------------------------------------------------------------------

test("a removed item stays out of the list, even when the page fetches it again, until the tab goes elsewhere", async () => {
  const b = load({ tabs: [PAGE] });
  await b.chrome.storage.session.set({ "media-1": [
    { url: "https://cdn.example.com/a.mp4", kind: "video", size: 5e6, seen: 1, hidden: false },
    { url: "https://cdn.example.com/b.mp4", kind: "video", size: 5e6, seen: 2, hidden: false },
  ] });
  await b.ask({ type: "remove", tabId: 1, item: { kind: "video", url: "https://cdn.example.com/a.mp4" } });
  const shown = async () => plain((await b.get("listed")(1, PAGE, "oldest", false)).map((i) => i.url));
  assert.deepEqual(await shown(), ["https://cdn.example.com/b.mp4"]);
  await b.respond({ url: "https://cdn.example.com/a.mp4", type: "media", responseHeaders: headers({ "content-type": "video/mp4", "content-length": "5000000" }) });
  assert.deepEqual(await shown(), ["https://cdn.example.com/b.mp4"], "fetched again: still out");
  b.chrome.tabs.onUpdated.fire(1, { url: "https://www.example.com/next" });
  await b.get("noting");
  assert.deepEqual(await b.items(), [], "another page: a fresh list");
});

test("a video from the page's own data can be removed too", async () => {
  const mpd = (n) => `<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" mediaPresentationDuration="PT${n}S"><Period><AdaptationSet contentType="video"><Representation id="v" bandwidth="1" width="1280" height="720" codecs="avc1"><BaseURL>https://cdn.example.com/v${n}.mp4</BaseURL></Representation></AdaptationSet></Period></MPD>`;
  const b = load({ tabs: [PAGE], script: () => [mpd(24), mpd(1239)] });
  const shown = async () => plain((await b.get("listed")(1, PAGE, "smart")).filter((i) => i.kind === "page-dash").map((i) => i.seconds));
  assert.deepEqual(await shown(), [1239, 24]);
  await b.ask({ type: "remove", tabId: 1, item: { kind: "page-dash", manifest: mpd(24) } });
  assert.deepEqual(await shown(), [1239]);
});

test("Find Files hands the app every frame of the page as the browser shows it, with its cookies", async () => {
  const frames = [
    { url: "https://www.example.com/watch", html: "<html><iframe src=\"https://player.example.net/e/1\"></iframe></html>", resources: ["https://cdn.example.net/v/master.m3u8", "data:x"] },
    { url: "https://player.example.net/e/1", html: "<script>setup({file:\"https://cdn.example.net/v/master.m3u8\"})</script>", resources: ["https://cdn.example.net/v/master.m3u8"] },
    { url: "about:blank", html: "<html></html>", resources: [] },
  ];
  const b = load({ tabs: [PAGE], native: () => ({ ok: true, found: [] }), cookies: [{ name: "s", value: "1" }] });
  b.chrome.scripting.executeScript = async ({ target }) => (target.allFrames ? frames.map((result) => ({ result })) : []);
  const answer = await b.ask({ type: "scan", tabId: 1 });
  assert.equal(answer.ok, true);
  const sent = plain(b.sent.find((m) => m.command === "scan"));
  assert.deepEqual(sent.urls, [PAGE.url]);
  assert.deepEqual(sent.pages.map((p) => p.url), ["https://www.example.com/watch", "https://player.example.net/e/1"], "the browser's own blank frame is left out");
  assert.ok(sent.pages.every((p) => p.cookies === "s=1" && p.html.length > 0));
  assert.deepEqual(sent.resources, ["https://cdn.example.net/v/master.m3u8", "data:x"]);
});

test("Download Section hands the app the stream's from and to", async () => {
  const b = load({ tabs: [PAGE] });
  const url = "https://cdn.example.net/hls/long/master.m3u8";
  await b.ask({ type: "add", tabId: 1, item: { url, kind: "stream" }, start: true, from: "1:02:00", to: "1:10:00" });
  const sent = plain(b.sent.find((m) => m.command === "add"));
  assert.deepEqual([sent.from, sent.to, sent.stream, sent.queue], ["1:02:00", "1:10:00", true, false]);
  // A whole file isn't split: no section goes with it.
  await b.ask({ type: "add", tabId: 1, item: { url: "https://cdn.example.net/v.mp4", kind: "video" }, from: "0:10" });
  assert.equal(plain(b.sent.at(-1)).from, undefined);
});

test("a player's playlist named by a path part (/m3u8/<id>) goes with the player's own Referer", async () => {
  const b = load({ tabs: [PAGE] });
  const url = "https://player.example.org/m3u8/7075342";
  b.chrome.webRequest.onSendHeaders.fire({ tabId: 1, url, type: "xmlhttprequest", requestHeaders: headers({ Referer: "https://player.example.org/e/7" }) });
  await b.ask({ type: "add", tabId: 1, item: { url, kind: "stream" } });
  assert.equal(plain(b.sent.at(-1)).referer, "https://player.example.org/e/7");
  // One whose own address wasn't seen asked: the last headers its host was asked with in the tab.
  b.chrome.webRequest.onSendHeaders.fire({ tabId: 1, url: "https://cdn.example.org/seg/1.ts", type: "xmlhttprequest", requestHeaders: headers({ Referer: "https://player.example.org/e/8" }) });
  await b.ask({ type: "add", tabId: 1, item: { url: "https://cdn.example.org/play/abc", kind: "stream" } });
  assert.equal(plain(b.sent.at(-1)).referer, "https://player.example.org/e/8");
});

