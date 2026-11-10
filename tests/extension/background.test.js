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
  assert.deepEqual(b.sent.slice(2).map((m) => [m.urls[0], m.queue === true]), [["https://www.example.com/a.zip", true], ["https://www.example.com/b.zip", false]]);
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

test("another page in the tab forgets what the last one played", async () => {
  const b = load({ tabs: [PAGE] });
  await b.respond({ url: "https://cdn.example.com/m.mpd" });
  assert.equal((await b.items()).length, 1);
  b.chrome.tabs.onUpdated.fire(1, { url: "https://www.example.com/next" });
  await b.get("noting");
  assert.equal((await b.items()).length, 0);
});
