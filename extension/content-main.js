// Runs in the page's own world (SPECIFICATION.md §15): DASH manifests a page receives in its data replies instead of
// its scripts (Instagram's and Facebook's feeds load each reel's `video_dash_manifest` through GraphQL) are passed
// to content.js, which hands them to background.js, so the list and the page's button know those videos too. Only
// text replies that mention "MPD" are read, and the page sees no difference.

(() => {
  if (globalThis.__herbsDmManifests) return;
  globalThis.__herbsDmManifests = true;

  // The manifests in a reply's text: JSON strings holding `<MPD` (written `<`), or a manifest as it is.
  function manifestsIn(text) {
    const found = [];
    if (typeof text !== "string" || !text.includes("MPD")) return found;
    for (const m of text.matchAll(/"((?:[^"\\]|\\.)*?(?:\\u003C|<)MPD(?:[^"\\]|\\.)*?)"/g)) {
      try {
        const xml = JSON.parse(`"${m[1]}"`).trim();
        if (xml.includes("<MPD") && xml.length <= 2 * 1024 * 1024) found.push(xml);
      } catch {}
      if (found.length >= 20) break;
    }
    if (!found.length) for (const m of text.matchAll(/<MPD[\s\S]*?<\/MPD>/g)) found.push(m[0]);
    return found.slice(0, 20);
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
