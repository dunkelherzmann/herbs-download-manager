// The Herb's DM button on the files a page shows (SPECIFICATION.md §13, §15): while the pointer is over a sound, a
// picture (a background picture too), a video, an embedded document or a link to a file, a small button with the
// app's icon shows in its top right corner (beside a link too small to hold it); a click downloads that file with
// Herb's Download Manager (background.js finds it and hands it over). Runs in every page and frame.

(() => {
  if (globalThis.__herbsDmButton) return;
  globalThis.__herbsDmButton = true;
  const api = globalThis.browser ?? chrome;
  const SIZE = 22;
  // Pictures smaller than this (icons, avatars, spacers) get no button.
  const PICTURE_MIN = 48;

  // The button lives in its own shadow root, so the page's styles can't reach it.
  const host = document.createElement("herbs-dm-button");
  const root = host.attachShadow({ mode: "closed" });
  root.innerHTML = `<style>
    :host { all: initial; }
    button { position: fixed; z-index: 2147483647; width: ${SIZE}px; height: ${SIZE}px; padding: 0; margin: 0;
      border: 1px solid rgba(255,255,255,0.7); border-radius: 6px; background: rgba(28,28,30,0.78);
      box-shadow: 0 1px 4px rgba(0,0,0,0.4); cursor: pointer; display: none; place-items: center; }
    button.shown { display: grid; }
    button:hover { background: rgba(0,122,255,0.95); }
    button:focus-visible { outline: 2px solid #0a84ff; outline-offset: 1px; }
    img { width: 16px; height: 16px; display: block; pointer-events: none; }
    span { display: none; color: #fff; font: 700 13px/1 system-ui, -apple-system, "Segoe UI", sans-serif; }
    button.ok { background: #248a3d; } button.bad { background: #d70015; }
    button.ok img, button.bad img { display: none; } button.ok .ok, button.bad .bad { display: block; }
    button.busy img { opacity: 0.5; }
  </style><button type="button"><img alt=""><span class="ok">✓</span><span class="bad">!</span></button>`;
  const button = root.querySelector("button");
  root.querySelector("img").src = api.runtime.getURL("icons/icon-32.png");

  // Manifests the page received in its data replies (content-main.js), handed to background.js.
  addEventListener("message", (e) => {
    if (e.source !== window || !e.data || !Array.isArray(e.data.herbsDmManifests)) return;
    api.runtime.sendMessage({ type: "page-manifests", manifests: e.data.herbsDmManifests.filter((m) => typeof m === "string") }).catch(() => {});
  });

  let target = null;
  let hideTimer = 0;
  let resetTimer = 0;

  // What a link's or a file's address is, by its extension (as the app's page scan sorts files), or "" for none.
  const EXT = {
    video: "mp4 m4v webm mkv mov avi wmv flv f4v 3gp ogv mpg mpeg m2ts mts vob rmvb divx",
    audio: "mp3 m4a m4b aac ogg oga opus flac wav wma aiff mka ac3 ape",
    image: "jpg jpeg png gif webp avif heic tif tiff bmp",
    stream: "m3u8 mpd",
    file: "zip rar 7z tar gz tgz bz2 xz zst iso dmg img pdf doc docx xls xlsx ppt pptx odt ods odp epub rtf csv exe msi pkg deb rpm appimage apk flatpakref torrent",
  };
  const KIND_OF_EXT = new Map(Object.entries(EXT).flatMap(([kind, list]) => list.split(" ").map((e) => [e, kind])));
  const web = (url) => /^https?:/i.test(url || "");
  function kindOfUrl(url) {
    try {
      const last = new URL(url).pathname.split("/").pop();
      const ext = (/\.([a-z0-9]{2,9})$/i.exec(last) || [])[1];
      return (ext && KIND_OF_EXT.get(ext.toLowerCase())) || "";
    } catch {
      return "";
    }
  }
  const WORD = { video: "video", audio: "sound", image: "picture", stream: "video", file: "file" };
  const label = (t) => `Download this ${WORD[t.kind] || "file"} with Herb's Download Manager`;

  function sourceOf(el) {
    if (el.currentSrc || el.src) return el.currentSrc || el.src;
    const source = el.querySelector && el.querySelector("source[src]");
    return source ? source.src : "";
  }

  // A link to a file: its address names a file type, or the link says it downloads.
  function fileLink(el) {
    const a = el.closest && el.closest("a[href]");
    if (!a || !web(a.href)) return null;
    const kind = kindOfUrl(a.href) || (a.hasAttribute("download") ? "file" : "");
    return kind ? { el: a, src: a.href, kind, link: true } : null;
  }

  // A picture drawn as an element's background (not the page's own, nor a gradient).
  function backgroundOf(el) {
    if (el === document.body || el === document.documentElement) return null;
    const url = /url\(["']?([^"')]+)["']?\)/.exec(getComputedStyle(el).backgroundImage || "");
    if (!url) return null;
    const src = new URL(url[1], location.href).href;
    return web(src) ? { el, src, kind: "image" } : null;
  }

  // The file under the point: a link to a file first (a thumbnail that opens its full picture gives that picture),
  // then a sound, picture or video, an embedded document, then a background picture.
  function fileAt(x, y) {
    const els = document.elementsFromPoint(x, y).filter((el) => el !== host);
    const big = (el, min) => {
      const r = el.getBoundingClientRect();
      return r.width >= min && r.height >= min;
    };
    for (const el of els) {
      const link = fileLink(el);
      if (link) return link;
      if (el instanceof HTMLVideoElement || el instanceof HTMLAudioElement) {
        if (big(el, SIZE + 8)) return { el, src: sourceOf(el), kind: el instanceof HTMLVideoElement ? "video" : "audio" };
      } else if (el instanceof HTMLImageElement) {
        if (big(el, PICTURE_MIN) && web(el.currentSrc || el.src)) return { el, src: el.currentSrc || el.src, kind: "image" };
      } else if (el instanceof HTMLEmbedElement || el instanceof HTMLObjectElement) {
        const src = el.src || el.data || "";
        if (big(el, PICTURE_MIN) && web(src)) return { el, src, kind: kindOfUrl(src) || "file" };
      }
    }
    for (const el of els) {
      if (!big(el, PICTURE_MIN)) continue;
      const bg = backgroundOf(el);
      if (bg) return bg;
    }
    return null;
  }

  // In the element's top right corner (inside what's on screen of it); beside a link too small to hold it.
  function place() {
    if (!target || !target.el.isConnected) return hide(true);
    const r = target.el.getBoundingClientRect();
    if (r.bottom < 0 || r.top > innerHeight || r.right < 0 || r.left > innerWidth) return hide(true);
    let left;
    let top;
    if (r.height < SIZE + 12 || r.width < SIZE + 12) {
      left = r.right + 4;
      top = r.top + (r.height - SIZE) / 2;
    } else {
      left = Math.min(r.right, innerWidth) - 6 - SIZE;
      top = Math.max(r.top, 0) + 6;
    }
    button.style.left = `${Math.max(0, Math.min(left, innerWidth - SIZE))}px`;
    button.style.top = `${Math.max(0, Math.min(top, innerHeight - SIZE))}px`;
  }

  function show(found) {
    clearTimeout(hideTimer);
    const same = target && target.el === found.el && target.src === found.src;
    // While a click is being handed over, the button stays with its element.
    if (!same && button.classList.contains("busy")) return;
    if (!same) {
      target = found;
      button.className = "";
      button.title = label(found);
      button.setAttribute("aria-label", label(found));
    }
    if (!host.isConnected) (document.body || document.documentElement).append(host);
    button.classList.add("shown");
    place();
  }

  function hide(now) {
    clearTimeout(hideTimer);
    const go = () => {
      if (button.matches(":hover") || button.classList.contains("busy")) return;
      button.classList.remove("shown");
      target = null;
    };
    if (now) go();
    else hideTimer = setTimeout(go, 800);
  }

  // Looked for at most once a frame while the pointer moves; players often lay their controls over the video, so
  // everything under the pointer is checked, not just the top element.
  let pending = false;
  addEventListener("mousemove", (e) => {
    if (pending) return;
    pending = true;
    requestAnimationFrame(() => {
      pending = false;
      if (e.composedPath().includes(host)) return;
      const found = fileAt(e.clientX, e.clientY);
      if (found) show(found);
      else if (target) hide(false);
    });
  }, { passive: true, capture: true });
  addEventListener("scroll", () => target && place(), { passive: true, capture: true });
  addEventListener("resize", () => target && place(), { passive: true });
  document.addEventListener("mouseleave", () => target && hide(false));

  // Swallowed before the page sees it, so a click doesn't also play, pause or open what's under it.
  for (const type of ["mousedown", "mouseup", "pointerdown", "pointerup", "dblclick"]) {
    button.addEventListener(type, (e) => e.stopPropagation());
  }
  button.addEventListener("click", async (e) => {
    e.preventDefault();
    e.stopPropagation();
    if (!target || button.classList.contains("busy")) return;
    const { el, src, kind } = target;
    const player = {
      // A video's or sound's address as it plays now (it may have changed since the pointer came).
      src: el instanceof HTMLMediaElement ? sourceOf(el) : src,
      kind,
      seconds: el instanceof HTMLMediaElement && Number.isFinite(el.duration) ? el.duration : 0,
      width: el instanceof HTMLVideoElement ? el.videoWidth : 0,
      height: el instanceof HTMLVideoElement ? el.videoHeight : 0,
    };
    clearTimeout(resetTimer);
    button.className = "shown busy";
    let answer;
    try {
      answer = await api.runtime.sendMessage({ type: "page-button", player });
    } catch {
      answer = { ok: false, error: "Herb's Download Manager's extension was updated: load the page again." };
    }
    const ok = answer && answer.ok;
    button.className = `shown ${ok ? "ok" : "bad"}`;
    const words = ok ? "Downloading with Herb's Download Manager" : (answer && answer.error) || "Herb's Download Manager didn't answer.";
    button.title = words;
    button.setAttribute("aria-label", words);
    resetTimer = setTimeout(() => {
      button.className = target ? "shown" : "";
      if (target) {
        button.title = label(target);
        button.setAttribute("aria-label", label(target));
      }
      if (!button.matches(":hover")) hide(false);
    }, ok ? 2000 : 5000);
  }, true);
})();
