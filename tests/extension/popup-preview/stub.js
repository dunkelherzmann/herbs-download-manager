// A stand-in browser for looking at the list outside the browser (not shipped).
const items = [
  { kind: "youtube", url: "https://www.youtube.com/watch?v=8jPQjjsBbIc", videoId: "8jPQjjsBbIc", title: "How to stay calm when you know you'll be stressed | Daniel Levitin | TED",
    author: "TED", seconds: 740, quality: "720p", qualities: [{ lines: 720, size: 99903265 }, { lines: 480, size: 43436061 }, { lines: 360, size: 21378523 }], sound: 11984389 },
  { kind: "stream", url: "https://cdn.example.net/hls/feed/master.m3u8", seconds: 5011,
    qualities: [{ lines: 1080, bandwidth: 5200000 }, { lines: 798, bandwidth: 2600000 }, { lines: 480, bandwidth: 900000 }],
    sounds: [{ language: "en", name: "English" }, { language: "es", name: "Español" }] },
  { kind: "page-dash", manifest: "<MPD/>", quality: "720p", seconds: 127, sound: true },
  { kind: "video", url: "https://media.example.org/clips/Morning%20walk%20by%20the%20river.mp4", size: 48200000 },
  { kind: "audio", url: "https://media.example.org/talks/interview.mp3", size: 7300000 },
];
globalThis.chrome = {
  runtime: { getManifest: () => ({ version: "1.29" }), sendMessage: async (m) => {
    if (m.type === "state") return { on: true, action: "download", order: "smart", panelOffered: true, panel: false, items, site: "example.com" };
    if (m.type === "scan") return { ok: true, found: [
      { url: "https://example.com/files/guide.pdf", name: "Travel guide.pdf", size: 2400000, kind: "Document", stream: false },
      { url: "https://example.com/files/photo.jpg", name: "Harbour at dawn.jpg", size: 380000, kind: "Picture", stream: false },
      { url: "https://example.com/files/setup.zip", name: "Setup files.zip", size: 152000000, kind: "Archive", stream: false },
    ] };
    if (m.type === "set-on") return { on: m.on };
    if (m.type === "name") return { name: "Feed 2026.mp4" };
    if (m.type === "preview") return { ok: false };
    return { ok: true };
  } },
  tabs: { query: async () => [{ id: 1, url: "https://www.example.com/watch/feed", title: "Feed 2026 — Watch Online" }],
          onActivated: { addListener() {} }, onUpdated: { addListener() {}, removeListener() {} } },
  storage: { session: { onChanged: { addListener() {} } } },
};
