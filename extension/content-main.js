// Runs in the page's own world (SPECIFICATION.md §15): DASH manifests a page receives in its data replies instead of
// its scripts (Instagram's and Facebook's feeds load each reel's `video_dash_manifest` through GraphQL) are passed
// to content.js, which hands them to background.js, so the list and the page's button know those videos too. Only
// text replies that mention "MPD" are read, and the page sees no difference.

(() => {
  if (globalThis.__herbsDmManifests) return;
  globalThis.__herbsDmManifests = true;

  // The manifests in a reply's text, each with the ids of the post or story that holds it (its `pk`, `id` and `code`,
  // and those of the objects around it): from its JSON (Instagram's replies may start with `for (;;);` or hold
  // several, a line each), else JSON strings holding `<MPD` (written `\u003C` or `\u003c`), else as it is.
  function manifestsIn(text) {
    const found = [];
    if (typeof text !== "string" || !text.includes("MPD")) return found;
    const seen = new Set();
    const keep = (xml, ids = []) => {
      const t = xml.trim();
      const key = `${t.length}:${t.slice(-300)}`;
      if (!t.includes("<MPD") || t.length > 2 * 1024 * 1024 || seen.has(key) || found.length >= 30) return;
      seen.add(key);
      found.push({ xml: t, ids });
    };
    const idsOf = (o) => [o.pk, o.id, o.code].filter((v) => typeof v === "string" || typeof v === "number").flatMap((v) => [String(v), String(v).split("_")[0]]);
    const walk = (o, depth, around) => {
      if (!o || typeof o !== "object" || depth > 80 || found.length >= 30) return;
      const ids = Array.isArray(o) ? around : [...idsOf(o), ...around];
      for (const v of Array.isArray(o) ? o : Object.values(o)) {
        if (typeof v === "string" && v.includes("<MPD")) keep(v, ids);
        else if (v && typeof v === "object") walk(v, depth + 1, ids);
      }
    };
    for (const part of text.replace(/^\s*for \(;;\);/, "").split(/\r?\n(?=\s*[{[])/)) {
      try {
        walk(JSON.parse(part), 0, []);
      } catch {}
    }
    if (found.length) return found;
    for (const m of text.matchAll(/"((?:[^"\\]|\\.)*?(?:\\u003[cC]|<)MPD(?:[^"\\]|\\.)*?)"/g)) {
      try {
        keep(JSON.parse(`"${m[1]}"`));
      } catch {}
    }
    if (!found.length) for (const m of text.matchAll(/<MPD[\s\S]*?<\/MPD>/g)) keep(m[0]);
    return found;
  }

  function pass(text) {
    const manifests = manifestsIn(text);
    if (manifests.length) window.postMessage({ herbsDmManifests: manifests }, location.origin === "null" ? "*" : location.origin);
  }

  const send = XMLHttpRequest.prototype.send;
  XMLHttpRequest.prototype.send = function (...args) {
    this.addEventListener("load", () => {
      try {
        if (this.responseType === "" || this.responseType === "text") pass(this.responseText);
      } catch {}
    });
    return send.apply(this, args);
  };

  // fetch: only JSON or script replies, read from a copy, under 20 MB.
  const originalFetch = window.fetch;
  if (typeof originalFetch === "function") {
    window.fetch = function (...args) {
      const answer = originalFetch.apply(this, args);
      answer.then((r) => {
        try {
          const type = r.headers.get("content-type") || "";
          const length = Number(r.headers.get("content-length") || 0);
          if (/json|javascript/.test(type) && length < 20 * 1024 * 1024) r.clone().text().then(pass, () => {});
        } catch {}
      }, () => {});
      return answer;
    };
  }

  globalThis.__herbsDmManifestsIn = manifestsIn;
})();
