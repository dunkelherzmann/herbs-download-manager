// The Herb's DM button on a page's sounds, pictures and videos (SPECIFICATION.md §15): while the pointer is over one,
// a small button with the app's icon shows in its bottom right corner; a click downloads that element's own file
// with Herb's Download Manager (background.js finds it and hands it over). Runs in every page and frame.

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

  let target = null;
  let hideTimer = 0;
  let resetTimer = 0;

  const kindOf = (el) => (el instanceof HTMLVideoElement ? "video" : el instanceof HTMLAudioElement ? "audio" : "image");
  const label = (el) => `Download this ${{ video: "video", audio: "sound", image: "picture" }[kindOf(el)]} with Herb's Download Manager`;

  // A sound, picture or video under the point, big enough to carry the button.
  function mediaAt(x, y) {
    for (const el of document.elementsFromPoint(x, y)) {
      if (el === host) continue;
      if (!(el instanceof HTMLVideoElement || el instanceof HTMLAudioElement || el instanceof HTMLImageElement)) continue;
      const r = el.getBoundingClientRect();
      if (r.width < SIZE + 8 || r.height < SIZE + 4) continue;
      if (el instanceof HTMLImageElement && (r.width < PICTURE_MIN || r.height < PICTURE_MIN || !(el.currentSrc || el.src))) continue;
      return el;
    }
    return null;
  }

  // In the element's bottom right corner (inside what's on screen of it).
  function place() {
    if (!target || !target.isConnected) return hide(true);
    const r = target.getBoundingClientRect();
    const right = Math.min(r.right, innerWidth) - 6;
    const bottom = Math.min(r.bottom, innerHeight) - 6;
    if (right - SIZE < Math.max(r.left, 0) || bottom - SIZE < Math.max(r.top, 0)) return hide(true);
    button.style.left = `${right - SIZE}px`;
    button.style.top = `${bottom - SIZE}px`;
  }

  function show(el) {
    clearTimeout(hideTimer);
    // While a click is being handed over, the button stays with its element.
    if (el !== target && button.classList.contains("busy")) return;
    if (el !== target) {
      target = el;
      button.className = "";
      button.title = label(el);
      button.setAttribute("aria-label", label(el));
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
      const el = mediaAt(e.clientX, e.clientY);
      if (el) show(el);
      else if (target) hide(false);
    });
  }, { passive: true, capture: true });
  addEventListener("scroll", () => target && place(), { passive: true, capture: true });
  addEventListener("resize", () => target && place(), { passive: true });
  document.addEventListener("mouseleave", () => target && hide(false));

  function sourceOf(el) {
    if (el.currentSrc || el.src) return el.currentSrc || el.src;
    const source = el.querySelector && el.querySelector("source[src]");
    return source ? source.src : "";
  }

  // Swallowed before the page sees it, so a click doesn't also play, pause or open what's under it.
  for (const type of ["mousedown", "mouseup", "pointerdown", "pointerup", "dblclick"]) {
    button.addEventListener(type, (e) => e.stopPropagation());
  }
  button.addEventListener("click", async (e) => {
    e.preventDefault();
    e.stopPropagation();
    if (!target || button.classList.contains("busy")) return;
    const el = target;
    const player = {
      src: sourceOf(el),
      kind: kindOf(el),
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
