// YouTube (SPECIFICATION.md §15): YouTube's player streams a video only through its own protocol (SABR: POST requests
// with protobuf bodies, answers in UMP parts), and its addresses need a token its player makes. This script runs in
// YouTube's pages, in the page's own world: it learns the player's streaming address (with its `n` already worked out
// by the player) and its token from the player's own requests, then downloads a video's picture and sound with the
// same protocol, handing each piece to content.js, which passes it to Herb's Download Manager. The protocol's layout
// follows the open-source googlevideo project (MIT).

(() => {
  if (globalThis.__herbsDmYouTube) return;
  globalThis.__herbsDmYouTube = true;

  // ---- protobuf: just what the protocol needs ---------------------------------------------------------------------

  // Writing: a message is a list of [field, value]; a value is a number (varint), a boolean, a string, bytes
  // (Uint8Array), or a nested message (an array of pairs). Fields with undefined values are left out.
  function encode(fields) {
    const out = [];
    const varint = (n) => {
      let v = BigInt(n);
      do {
        let b = Number(v & 0x7fn);
        v >>= 7n;
        if (v) b |= 0x80;
        out.push(b);
      } while (v);
    };
    for (const [field, value] of fields) {
      if (value === undefined || value === null) continue;
      if (typeof value === "number" || typeof value === "bigint" || typeof value === "boolean") {
        varint(field * 8);
        varint(typeof value === "boolean" ? Number(value) : value);
      } else {
        const bytes = typeof value === "string" ? new TextEncoder().encode(value) : Array.isArray(value) ? encode(value) : value;
        varint(field * 8 + 2);
        varint(bytes.length);
        for (const b of bytes) out.push(b);
      }
    }
    return new Uint8Array(out);
  }

  // Reading: field → list of values (a number for varints, a Uint8Array for bytes and messages).
  function decode(bytes) {
    const fields = new Map();
    let i = 0;
    const varint = () => {
      let v = 0n;
      let shift = 0n;
      for (;;) {
        if (i >= bytes.length) throw new Error("truncated");
        const b = bytes[i++];
        v |= BigInt(b & 0x7f) << shift;
        shift += 7n;
        if (!(b & 0x80)) return v;
      }
    };
    while (i < bytes.length) {
      const key = Number(varint());
      const field = key >> 3;
      const wire = key & 7;
      let value;
      if (wire === 0) value = Number(varint());
      else if (wire === 1) { value = bytes.subarray(i, i + 8); i += 8; }
      else if (wire === 5) { value = bytes.subarray(i, i + 4); i += 4; }
      else if (wire === 2) { const n = Number(varint()); value = bytes.subarray(i, i + n); i += n; }
      else throw new Error(`wire type ${wire}`);
      if (!fields.has(field)) fields.set(field, []);
      fields.get(field).push(value);
    }
    return fields;
  }
  const first = (fields, n) => (fields.get(n) || [])[0];
  const text = (bytes) => (bytes ? new TextDecoder().decode(bytes) : "");

  // ---- what the player itself sends ----------------------------------------------------------------------------------

  // The player's streaming requests, by the streaming settings they carry (one video's, or an advert's): the last one's
  // address (with `n` worked out) and its body's streamer context (who the client is, its token).
  const seenBy = new Map();
  const settingsKey = (bytes) => `${bytes.length}:${Array.from(bytes.subarray(0, 48)).join(",")}:${Array.from(bytes.subarray(-48)).join(",")}`;
  function note(url, body) {
    try {
      const u = new URL(url, location.href);
      if (!/\/videoplayback$/.test(u.pathname) || !body) return;
      const bytes = body instanceof Uint8Array ? body : body instanceof ArrayBuffer ? new Uint8Array(body) : ArrayBuffer.isView(body) ? new Uint8Array(body.buffer, body.byteOffset, body.byteLength) : null;
      if (!bytes) return;
      const fields = decode(bytes);
      const context = first(fields, 19);
      const settings = first(fields, 5);
      if (!context || !settings) return;
      u.searchParams.delete("rn");
      const key = settingsKey(settings);
      seenBy.delete(key);
      seenBy.set(key, { url: u.href, context: new Uint8Array(context), at: Date.now() });
      if (seenBy.size > 12) seenBy.delete(seenBy.keys().next().value);
    } catch {}
  }
  const originalFetch = window.fetch;
  window.fetch = function (input, init) {
    try {
      const url = typeof input === "string" ? input : input && input.url;
      if (url && url.includes("/videoplayback")) {
        // The player passes a Request that holds its method and body: read the body from a copy.
        if (input instanceof Request && input.method === "POST") input.clone().arrayBuffer().then((b) => note(url, b), () => {});
        else if (init && String(init.method).toUpperCase() === "POST") note(url, init.body);
      }
    } catch {}
    return originalFetch.apply(this, arguments);
  };
  const open = XMLHttpRequest.prototype.open;
  const send = XMLHttpRequest.prototype.send;
  XMLHttpRequest.prototype.open = function (method, url) {
    this.__herbsDm = method === "POST" && String(url).includes("/videoplayback") ? String(url) : null;
    return open.apply(this, arguments);
  };
  XMLHttpRequest.prototype.send = function (body) {
    if (this.__herbsDm) note(this.__herbsDm, body);
    return send.apply(this, arguments);
  };

  // ---- the video on the page ---------------------------------------------------------------------------------------

  // The video the page's address names: `watch?v=<id>`, `/shorts/<id>`, `/live/<id>`, or an embedded player's
  // `/embed/<id>`.
  function pageVideoId() {
    const v = new URLSearchParams(location.search).get("v");
    if (v) return v;
    const m = /^\/(shorts|live|embed)\/([\w-]{6,})/.exec(location.pathname);
    return m ? m[2] : "";
  }

  // The player's answer for that video: Shorts play in their own player (`#shorts-player`), watch pages in
  // `#movie_player`; the one playing the address's video wins, else one that plays something.
  function playerResponse() {
    const wanted = pageVideoId();
    const players = [document.getElementById("shorts-player"), document.getElementById("movie_player"), ...document.querySelectorAll(".html5-video-player")]
      .filter((p, i, all) => p && all.indexOf(p) === i && typeof p.getPlayerResponse === "function");
    const answers = [];
    for (const p of players) {
      try {
        const r = p.getPlayerResponse();
        if (r && r.videoDetails) answers.push(r);
      } catch {}
    }
    const initial = window.ytInitialPlayerResponse;
    if (initial && initial.videoDetails) answers.push(initial);
    return answers.find((r) => wanted && r.videoDetails.videoId === wanted) || answers[0] || null;
  }

  // The formats to choose from: MP4 pictures (H.264 first at each size, then AV1) and MP4 sound (the default track).
  function choices(response) {
    const formats = ((response && response.streamingData) || {}).adaptiveFormats || [];
    const codec = (f) => ((/codecs="([^".]+)/.exec(f.mimeType || "") || [])[1] || "");
    const videos = formats.filter((f) => /^video\/mp4/.test(f.mimeType) && !f.drmFamilies && /^(avc1|av01)$/.test(codec(f)))
      .map((f) => ({ ...f, codec: codec(f), lines: Math.min(f.width || 0, f.height || 0) }))
      .sort((a, b) => b.lines - a.lines || (a.codec === "avc1" ? -1 : 1) - (b.codec === "avc1" ? -1 : 1) || (b.fps || 0) - (a.fps || 0));
    const sounds = formats.filter((f) => /^audio\/mp4/.test(f.mimeType) && !f.drmFamilies && (!f.audioTrack || f.audioTrack.audioIsDefault))
      .sort((a, b) => (b.bitrate || 0) - (a.bitrate || 0));
    return { videos, sounds };
  }

  // The player's request for this video (not an advert's): the one carrying the video's own streaming settings.
  function settingsOf(response) {
    const config = response && (((response.playerConfig || {}).mediaCommonConfig || {}).mediaUstreamerRequestConfig || {}).videoPlaybackUstreamerConfig;
    return config ? Uint8Array.from(atob(config.replace(/-/g, "+").replace(/_/g, "/")), (c) => c.charCodeAt(0)) : null;
  }
  function playerRequest(response) {
    const settings = settingsOf(response);
    return settings ? seenBy.get(settingsKey(settings)) || null : null;
  }

  // What the list shows about the video: its id, title, length and qualities (one per size, H.264 when there is).
  function describe() {
    const response = playerResponse();
    const details = (response && response.videoDetails) || {};
    const { videos, sounds } = choices(response);
    const lines = [];
    for (const v of videos) if (!lines.some((l) => l.lines === v.lines)) lines.push({ lines: v.lines, codec: v.codec, size: Number(v.contentLength) || 0 });
    return {
      id: details.videoId || "",
      title: details.title || document.title.replace(/ - YouTube$/, ""),
      author: details.author || "",
      seconds: Number(details.lengthSeconds) || 0,
      live: !!details.isLiveContent && !!details.isLive,
      qualities: lines,
      sound: sounds[0] ? Number(sounds[0].contentLength) || 0 : 0,
      ready: !!(playerRequest(response) && (response.streamingData || {}).serverAbrStreamingUrl),
    };
  }

  // ---- SABR ----------------------------------------------------------------------------------------------------------

  // UMP: parts one after another, each a type and a size in UMP's own variable-length numbers, then its bytes.
  function umpNumber(b, i) {
    if (i >= b.length) return null;
    const x = b[i];
    const n = x < 128 ? 1 : x < 192 ? 2 : x < 224 ? 3 : x < 240 ? 4 : 5;
    if (i + n > b.length) return null;
    let v;
    if (n === 1) v = x;
    else if (n === 2) v = (x & 0x3f) | (b[i + 1] << 6);
    else if (n === 3) v = (x & 0x1f) | (b[i + 1] << 5) | (b[i + 2] << 13);
    else if (n === 4) v = (x & 0x0f) | (b[i + 1] << 4) | (b[i + 2] << 12) | (b[i + 3] << 20);
    else v = (b[i + 1] | (b[i + 2] << 8) | (b[i + 3] << 16)) + b[i + 4] * 2 ** 24;
    return [v, i + n];
  }
  function umpParts(bytes) {
    const parts = [];
    let i = 0;
    for (;;) {
      const type = umpNumber(bytes, i);
      if (!type) break;
      const size = umpNumber(bytes, type[1]);
      if (!size || size[1] + size[0] > bytes.length) break;
      parts.push([type[0], bytes.subarray(size[1], size[1] + size[0])]);
      i = size[1] + size[0];
    }
    return parts;
  }

  const formatId = (f) => [[1, f.itag], [2, f.lastModified ? BigInt(f.lastModified) : undefined], [3, f.xtags]];
  const sameFormat = (fields, f) => first(fields, 1) === f.itag;

  // Downloads the chosen picture (`lines`, or the best) and sound, calling `piece(track, bytes, init)` in order for each
  // track (0 picture, 1 sound) and `progress(bytes)`; `stopped()` ends it early. Resolves to the tracks' sizes.
  async function download({ lines, soundOnly }, piece, progress, stopped) {
    const response = playerResponse();
    const streaming = (response && response.streamingData) || {};
    if (!streaming.serverAbrStreamingUrl) throw new Error("This video can't be downloaded: YouTube gave no stream for it.");
    const details = response.videoDetails || {};
    const configBytes = settingsOf(response);
    if (!configBytes) throw new Error("YouTube's player gave no streaming settings for this video.");
    const seen = playerRequest(response);
    if (!seen) throw new Error("Play the video for a moment first (after any advert), so YouTube's player starts streaming it.");
    const { videos, sounds } = choices(response);
    const video = soundOnly ? null : videos.find((v) => !lines || v.lines <= lines) || videos[videos.length - 1];
    const sound = sounds[0];
    if (!sound || (!soundOnly && !video)) throw new Error("This video has no MP4 picture and sound to download.");
    const tracks = [video, sound].map((f, i) => f && { format: f, index: i, init: false, initDone: false, last: 0, endMs: 0, startMs: 0, end: 0, bytes: 0 });
    const active = tracks.filter(Boolean);

    // The player's streamer context with our own playback cookie and contexts.
    const base = decode(seen.context);
    let url = new URL(seen.url);
    let cookie = null;
    let backoff = 0;
    const contexts = new Map();
    const sending = new Set();
    let request = 0;
    const started = Date.now();
    let received = 0;
    let rejected = 0;
    let idle = 0;
    // A test can ask for a trace of each request (what was asked, which parts came back).
    const trace = Array.isArray(globalThis.__herbsDmTrace) ? globalThis.__herbsDmTrace : null;

    // Segments arrive by header id: [track, segment number, is init, chunks].
    for (let rounds = 0; rounds < 100000; rounds++) {
      if (stopped()) return null;
      if (active.every((t) => t.end && t.last >= t.end)) break;
      if (backoff) await new Promise((r) => setTimeout(r, Math.min(backoff, 10000)));
      const elapsed = Date.now() - started;
      // The player's place: where the track that's furthest behind has its pieces up to.
      const playerMs = Math.min(...active.map((t) => t.endMs));
      const abr = [
        [16, video ? video.lines : undefined], [21, video ? video.lines : undefined], [23, 20000000], [28, playerMs],
        [29, elapsed], [34, 2], [36, elapsed], [39, elapsed], [40, soundOnly ? 1 : 0], [46, false], [71, true], [85, false],
      ];
      const context = [
        [1, first(base, 1)], [2, first(base, 2)], [3, cookie || undefined],
        ...[...contexts.values()].filter((c) => sending.has(c.type)).map((c) => [5, [[1, c.type], [2, c.value]]]),
        ...[...contexts.keys()].filter((type) => !sending.has(type)).map((type) => [6, type]),
      ];
      const body = encode([
        [1, abr],
        ...active.filter((t) => t.init).map((t) => [2, formatId(t.format)]),
        ...active.filter((t) => t.last > 0).map((t) => [3, [[1, formatId(t.format)], [2, t.startMs || undefined], [3, t.endMs - t.startMs], [4, 1], [5, t.last],
          [6, [[1, t.startMs], [2, t.endMs - t.startMs], [3, 1000]]]]]),
        [5, configBytes],
        [16, formatId(sound)],
        ...(video ? [[17, formatId(video)]] : []),
        [19, context],
      ]);
      request += 1;
      url.searchParams.set("rn", String(request));
      const answer = await originalFetch(url.href, {
        method: "POST", body, credentials: "include",
        headers: { "content-type": "application/x-protobuf", accept: "application/vnd.yt-ump" },
      });
      if (!answer.ok) throw new Error(`YouTube refused the stream (${answer.status}).`);
      const bytes = new Uint8Array(await answer.arrayBuffer());
      backoff = 0;
      if (trace) {
        const sps = umpParts(bytes).find(([t]) => t === 58);
        trace.push({ rn: request, playerMs, last: active.map((t) => t.last), end: active.map((t) => t.end), token: (first(base, 2) || []).length,
          protection: sps ? first(decode(sps[1]), 1) : null, parts: umpParts(bytes).map(([t, d]) => `${t}:${d.length}`).join(" ").slice(0, 120) });
      }
      const headers = new Map();
      let gotSomething = false;
      for (const [type, data] of umpParts(bytes)) {
        if (type === 20) { // MEDIA_HEADER
          const h = decode(data);
          const itag = first(h, 3);
          const track = active.find((t) => t.format.itag === itag);
          if (!track) continue;
          headers.set(first(h, 1) || 0, { track, init: !!first(h, 8), segment: first(h, 9) || 0, startMs: first(h, 11) || 0, durationMs: first(h, 12) || 0, chunks: [] });
        } else if (type === 21) { // MEDIA: its header's id, then data
          const head = headers.get(data[0]);
          if (head) head.chunks.push(data.subarray(1));
        } else if (type === 22) { // MEDIA_END
          const head = headers.get(data[0]);
          headers.delete(data[0]);
          if (!head) continue;
          const t = head.track;
          const size = head.chunks.reduce((n, c) => n + c.length, 0);
          const joined = new Uint8Array(size);
          let at = 0;
          for (const c of head.chunks) { joined.set(c, at); at += c.length; }
          if (head.init) {
            if (t.initDone) continue;
            t.initDone = true;
            await piece(t.index, joined, true);
          } else {
            // In order, once each: a piece already had, or one past a gap, is left (the next request asks again).
            if (head.segment !== t.last + 1) continue;
            t.last = head.segment;
            if (t.last === 1) t.startMs = head.startMs;
            t.endMs = head.startMs + head.durationMs;
            await piece(t.index, joined, false);
          }
          gotSomething = true;
          t.bytes += size;
          received += size;
          progress(received);
        } else if (type === 42) { // FORMAT_INITIALIZATION_METADATA
          const m = decode(data);
          const id = first(m, 2) && decode(first(m, 2));
          const t = id && active.find((t) => sameFormat(id, t.format));
          if (t) { t.init = true; t.end = first(m, 4) || 0; }
        } else if (type === 35) { // NEXT_REQUEST_POLICY
          const p = decode(data);
          cookie = first(p, 7) ? new Uint8Array(first(p, 7)) : cookie;
          backoff = first(p, 4) || 0;
        } else if (type === 43) { // SABR_REDIRECT
          const target = text(first(decode(data), 1));
          if (target) url = new URL(target);
        } else if (type === 44) { // SABR_ERROR
          const e = decode(data);
          throw new Error(`YouTube's stream failed (${text(first(e, 1))} ${first(e, 2) || ""}).`.replace(" )", ")"));
        } else if (type === 57) { // SABR_CONTEXT_UPDATE
          const c = decode(data);
          const kind = first(c, 1);
          const value = first(c, 3);
          if (kind === undefined || !value || !value.length) continue;
          if (first(c, 5) === 2 && contexts.has(kind)) continue;
          contexts.set(kind, { type: kind, value: new Uint8Array(value) });
          if (first(c, 4)) sending.add(kind);
        } else if (type === 59) { // SABR_CONTEXT_SENDING_POLICY
          const p = decode(data);
          for (const k of p.get(1) || []) sending.add(k);
          for (const k of p.get(2) || []) sending.delete(k);
          for (const k of p.get(3) || []) contexts.delete(k);
        } else if (type === 58) { // STREAM_PROTECTION_STATUS: 3 = the token was refused
          const s = decode(data);
          if (first(s, 1) === 3 && ++rejected > 3) throw new Error("YouTube refused this browser's token: play the video for a moment, then try again.");
        }
      }
      idle = gotSomething ? 0 : idle + 1;
      if (idle >= 30) throw new Error("YouTube stopped sending the stream.");
      if (!gotSomething && !backoff) backoff = 500;
    }
    return { video: tracks[0] && tracks[0].bytes, sound: tracks[1].bytes, title: details.title || "" };
  }

  // ---- talking to content.js ---------------------------------------------------------------------------------------

  // content.js asks (`herbsDmYouTubeAsk`) and gets answers (`herbsDmYouTubeAnswer`), each with the question's id; a
  // download's pieces go out as `piece` answers, each acknowledged (`herbsDmYouTubeAck`) before the next is sent.
  const acks = new Map();
  const stops = new Set();
  const post = (message, transfer) => window.postMessage({ herbsDmYouTubeAnswer: message }, location.origin, transfer || []);
  window.addEventListener("message", async (e) => {
    if (e.source !== window || !e.data) return;
    if (e.data.herbsDmYouTubeAck) {
      const { id, ok } = e.data.herbsDmYouTubeAck;
      const done = acks.get(id);
      acks.delete(id);
      if (done) done(ok);
      return;
    }
    const ask = e.data.herbsDmYouTubeAsk;
    if (!ask) return;
    if (ask.type === "describe") post({ id: ask.id, type: "describe", video: describe() });
    else if (ask.type === "stop") stops.add(ask.download);
    else if (ask.type === "download") {
      let n = 0;
      const piece = (track, bytes, init) => new Promise((resolve, reject) => {
        const id = `${ask.id}:${n++}`;
        acks.set(id, (ok) => (ok ? resolve() : reject(new Error("stopped"))));
        const copy = bytes.slice().buffer;
        post({ id: ask.id, type: "piece", ack: id, track, init, data: copy }, [copy]);
      });
      try {
        const sizes = await download(ask.options || {}, piece, (bytes) => post({ id: ask.id, type: "progress", bytes }), () => stops.has(ask.id));
        post({ id: ask.id, type: "done", stopped: !sizes, sizes });
      } catch (error) {
        post({ id: ask.id, type: "done", error: error && error.message === "stopped" ? "" : String((error && error.message) || error), stopped: error && error.message === "stopped" });
      }
    }
  });
})();
