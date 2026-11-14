// The toolbar button's list (SPECIFICATION.md §13, §15): whether the browser's downloads go to the app, the videos
// and sounds this tab played and the files found on the page, each added to the app with a click, or several at once
// by selecting them. background.js does the work.

const api = globalThis.browser ?? chrome;
const $ = (id) => document.getElementById(id);
const ask = (message) => api.runtime.sendMessage(message);

const KIND = { stream: "Video stream", dash: "Video stream", "page-dash": "Video", video: "Video", audio: "Sound" };

// "2:07", "1:02:03".
function length(seconds) {
  if (!seconds) return "";
  const s = Math.round(seconds);
  const hms = [Math.floor(s / 3600), Math.floor((s % 3600) / 60), s % 60];
  return hms[0] ? `${hms[0]}:${String(hms[1]).padStart(2, "0")}:${String(hms[2]).padStart(2, "0")}` : `${hms[1]}:${String(hms[2]).padStart(2, "0")}`;
}

function size(bytes) {
  if (!bytes) return "";
  const units = ["bytes", "KB", "MB", "GB"];
  let n = bytes;
  let u = 0;
  while (n >= 1000 && u < units.length - 1) {
    n /= 1000;
    u += 1;
  }
  return `${u ? n.toFixed(1) : n} ${units[u]}`;
}

function title(item, tab) {
  // The server's own name for it (Content-Disposition) when it gave one.
  if (item.name) return item.name;
  const last = decodeURIComponent(item.url.split(/[?#]/)[0].split("/").pop() || "");
  // A link with no file's name in it (TikTok's end in "/"): its page, as the app names it.
  if (!/\.[a-z0-9]{2,5}$/i.test(last) && tab && tab.title) return tab.title;
  return last || new URL(item.url).hostname;
}

// The tab it's about: the window's active one; with `?page=ADDRESS` (the popup opened in a tab of its own, for tests
// and pictures), the tab showing that address.
async function currentTab() {
  const page = new URLSearchParams(location.search).get("page");
  if (page) return (await api.tabs.query({})).find((t) => t.url === page);
  const [tab] = await api.tabs.query({ active: true, currentWindow: true });
  return tab;
}

// In the side panel (`?panel=1`) the list follows the window's active tab.
const inPanel = new URLSearchParams(location.search).has("panel");
if (inPanel) document.body.classList.add("panel");

// An icon button at work: its icon turns, and it can't be pressed again meanwhile.
function busy(button, on) {
  button.disabled = on;
  button.classList.toggle("busy", on);
}

// Small monochrome icons, drawn for this list (16 × 16, a line in the text's color).
const ICON = {
  pencil: '<path d="M10.8 2.7l2.5 2.5-7.6 7.6-3.2.7.7-3.2z"/><path d="M9.4 4.1l2.5 2.5"/>',
  down: '<circle cx="8" cy="8" r="6.2"/><path d="M8 4.6v6.2M5.4 8.4L8 11l2.6-2.6"/>',
  chevron: '<path d="M4 6l4 4 4-4"/>',
  close: '<path d="M4 4l8 8M12 4l-8 8"/>',
  play: '<path d="M5 3.5v9l7.5-4.5z"/>',
  sound: '<path d="M3 6.2h2.4L8.6 3.4v9.2L5.4 9.8H3z"/><path d="M11 5.6a3.4 3.4 0 0 1 0 4.8M12.8 4a5.6 5.6 0 0 1 0 8"/>',
  file: '<path d="M4 1.8h5.2L12.4 5v9.2H4z"/><path d="M9 1.8V5h3.4"/>',
};
function icon(name) {
  const box = document.createElement("span");
  box.innerHTML = `<svg viewBox="0 0 16 16" aria-hidden="true">${ICON[name]}</svg>`;
  return box.firstChild;
}

// The kind's short badge before a name, as video players label their formats: HLS, DASH, MP4, MP3…
function badge(item, label) {
  if (item.kind === "stream") return "HLS";
  if (item.kind === "dash" || item.kind === "page-dash") return "DASH";
  const ext = (label.match(/\.([a-z0-9]{2,5})$/i) || [])[1];
  return ext ? ext.toUpperCase() : item.kind === "audio" ? "SOUND" : item.kind === "video" ? "VIDEO" : "FILE";
}

// The items as last shown, so a refresh that changes nothing leaves the list (and its pickers) alone.
let shownKey = "";
// What a click on an item does on this site (its naming rule's action, §13).
let siteAction = "download";

async function show() {
  const tab = await currentTab();
  if (!tab) return;
  const state = await ask({ type: "state", tabId: tab.id }).catch(() => null);
  if (!state) return;
  // The switch sets "Take downloads" to its own state (never flips it), and isn't redrawn while that's under way.
  if (!switching) $("on").checked = state.on;
  $("on").onchange = async () => {
    switching = true;
    const answer = await ask({ type: "set-on", on: $("on").checked, tabId: tab.id }).catch(() => null);
    switching = false;
    if (answer) $("on").checked = answer.on;
  };
  gone = state.gone || [];
  $("min-size").value = String(state.minSize || 0);
  $("min-size").onchange = async () => {
    await ask({ type: "set-min-size", minSize: Number($("min-size").value) });
    showFound(tab);
  };
  siteAction = state.action || "download";
  $("rules").onclick = () => ask({ type: "rules", site: state.site }).then(() => window.close());
  $("order").value = state.order;
  $("order").onchange = async () => {
    await ask({ type: "set-order", order: $("order").value });
    shownKey = "";
    show();
  };
  $("panel-row").hidden = !state.panelOffered;
  $("panel").checked = state.panel;
  $("panel").onchange = async () => {
    await ask({ type: "set-panel", on: $("panel").checked });
  };
  // Reload: the tab loads again (the list starts afresh), and once it has, the page is looked through again.
  $("reload").onclick = async () => {
    const b = $("reload");
    b.disabled = true;
    b.textContent = "Reloading…";
    $("found").replaceChildren();
    $("found-heading").hidden = true;
    $("scan-note").hidden = true;
    lastFound = [];
    const loaded = loadedAgain(tab.id);
    await ask({ type: "look-again", tabId: tab.id });
    await loaded;
    b.disabled = false;
    b.textContent = "Reload";
    shownKey = "";
    scannedUrl = "";
    show();
  };
  // Select All (Select None once all are), and Delete All: every item off the list.
  $("select-all").onclick = () => {
    const boxes = [...document.querySelectorAll(".card .pick input")];
    const all = boxes.length > 0 && boxes.every((b) => b.checked);
    for (const b of boxes) b.checked = !all;
    selectionChanged();
  };
  $("delete-all").onclick = async () => {
    const urls = lastFound.map((f) => f.url);
    $("items").replaceChildren();
    $("found").replaceChildren();
    $("found-heading").hidden = true;
    $("scan-note").hidden = true;
    $("empty").hidden = false;
    selectionChanged();
    await ask({ type: "remove-all", tabId: tab.id, urls });
  };
  const key = `${tab.id}|${state.order}|${siteAction}|${state.items.map((i) => i.url || i.manifest.length).join("|")}`;
  if (key === shownKey) return;
  shownKey = key;
  const list = $("items");
  list.replaceChildren();
  $("empty").hidden = state.items.length > 0;
  for (const item of state.items) {
    const streamed = ["stream", "dash", "page-dash"].includes(item.kind);
    const facts = item.kind === "page-dash"
      ? [item.sound ? "Video with sound" : "Video", item.quality, length(item.seconds)]
      : [KIND[item.kind], size(item.size)];
    list.append(row(tab, item, streamed ? tab.title || title(item, tab) : title(item, tab), facts));
  }
  selectionChanged();
  $("scan").onclick = async () => {
    busy($("scan"), true);
    const note = $("scan-note");
    note.className = "note";
    note.textContent = "Looking through the page…";
    note.hidden = false;
    note.parentElement.querySelectorAll(":scope > button.open").forEach((b) => b.remove());
    const answer = await ask({ type: "scan", tabId: tab.id });
    busy($("scan"), false);
    note.hidden = true;
    const found = $("found");
    found.replaceChildren();
    $("found-heading").hidden = true;
    if (answer && answer.not_running) {
      // The app reads the page: offer to open it, then look again.
      note.className = "note";
      note.textContent = "Herb's Download Manager isn't running.";
      note.hidden = false;
      note.after(openButton("Open and Look Again", async () => {
        const opened = await ask({ type: "open" });
        if (opened && opened.ok) {
          note.hidden = true;
          $("scan").click();
        }
        return opened;
      }, note));
      return;
    }
    if (!answer || !answer.ok) {
      note.className = "note bad";
      note.textContent = (answer && answer.error) || "Herb's Download Manager didn't answer.";
      note.hidden = false;
      return;
    }
    lastFound = answer.found || [];
    showFound(tab);
  };
  // The page is looked through as soon as the list opens (and in the side panel, whenever its tab shows another
  // page), so its files and media are found without playing anything first.
  if (tab.url !== scannedUrl && /^https?:/.test(tab.url || "")) {
    scannedUrl = tab.url;
    $("scan").click();
  }
}
let scannedUrl = "";
// The files the page's last scan found, what was deleted from the list, and whether the switch is being set.
let lastFound = [];
let gone = [];
let switching = false;

// The files found on the page (the search results), without those deleted from the list or smaller than the size
// chosen beside their heading (a file of unknown size always shows).
function showFound(tab) {
  const found = $("found");
  const note = $("scan-note");
  found.replaceChildren();
  const min = Number($("min-size").value) || 0;
  const files = lastFound.filter((f) => !gone.includes(f.url));
  const shown = files.filter((f) => !min || !f.size || f.size >= min);
  $("found-heading").hidden = !files.length;
  note.hidden = shown.length > 0;
  note.className = "note";
  note.textContent = files.length ? `No files of ${$("min-size").selectedOptions[0].text} or more on this page.` : "No files or media found on this page.";
  for (const f of shown) {
    const item = { url: f.url, name: f.name, size: f.size || 0, kind: f.stream ? "stream" : "file", stream: f.stream, found: f.kind };
    found.append(row(tab, item, f.name, [f.kind, size(f.size)]));
  }
  selectionChanged();
}

// Resolves once the tab has loaded again (at most 30 s).
function loadedAgain(tabId) {
  return new Promise((done) => {
    let started = false;
    const finish = () => {
      api.tabs.onUpdated.removeListener(watch);
      clearTimeout(timer);
      done();
    };
    const watch = (id, change) => {
      if (id !== tabId) return;
      if (change.status === "loading") started = true;
      if (change.status === "complete" && started) finish();
    };
    const timer = setTimeout(finish, 30000);
    api.tabs.onUpdated.addListener(watch);
  });
}

// The bar of what's selected: how many, Add to Queue and Download; Select All says Select None once all are.
function selectionChanged() {
  const boxes = [...document.querySelectorAll(".card .pick input")];
  const picked = boxes.filter((b) => b.checked);
  $("selection").hidden = !picked.length;
  $("selected-count").textContent = `${picked.length} selected`;
  const all = boxes.length > 0 && picked.length === boxes.length;
  $("select-all").textContent = all ? "Select None" : "Select All";
  $("select-all").title = all ? "Select None: untick every item in the list (⌘A, Ctrl+A)" : "Select All: tick every item in the list (⌘A, Ctrl+A)";
  $("select-all").disabled = !boxes.length;
  $("delete-all").disabled = !document.querySelector(".card");
}
const pickedCards = () => [...document.querySelectorAll(".card")].filter((c) => c.querySelector(".pick input").checked);
$("download-selected").onclick = () => {
  for (const card of pickedCards()) card.addNow({ start: true });
};
$("queue-selected").onclick = () => {
  for (const card of pickedCards()) card.addNow({});
};
// ⌘A / Ctrl+A selects all (outside a text field), ⌘R / Ctrl+R reloads the tab rather than the list.
document.addEventListener("keydown", (e) => {
  if (!(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey) return;
  const typing = /^(INPUT|TEXTAREA)$/.test(e.target.tagName) && e.target.type !== "checkbox";
  const k = e.key.toLowerCase();
  if (k === "a" && !typing) {
    e.preventDefault();
    $("select-all").click();
  } else if (k === "r") {
    e.preventDefault();
    if (!$("reload").disabled) $("reload").click();
  }
});

// A stream's quality and sound pickers, when it offers a choice: "As in Settings" first, then each quality (picture
// lines and bit rate) and each sound language. `chosen()` gives what to send with the stream.
function pickers(item) {
  const box = document.createElement("div");
  box.className = "pickers";
  const list = (label, options) => {
    const select = document.createElement("select");
    select.className = "chip";
    select.setAttribute("aria-label", label);
    select.title = label;
    for (const [value, text] of options) select.append(new Option(text, value));
    box.append(select);
    return select;
  };
  const qualities = item.qualities || [];
  const sounds = item.sounds || [];
  const quality = qualities.length > 1 && list("Quality to download (Auto: as in Settings)", [["", "Auto quality"], ["highest", "Highest"],
    ...qualities.map((q) => [String(q.lines), `${q.lines}p${q.bandwidth ? ` · ${(q.bandwidth / 1e6).toFixed(1)} Mb/s` : ""}`])]);
  const sound = sounds.length > 1 && list("Sound to download (Auto: as in Settings)", [["*", "Auto sound"], ["", "The stream's own sound"],
    ...sounds.map((s) => [s.language, s.name === s.language ? s.language : `${s.name} (${s.language})`])]);
  const chosen = () => {
    const c = {};
    if (quality && quality.value) c.quality = quality.value;
    if (sound && sound.value !== "*") c.sound = sound.value;
    return c;
  };
  return { box: quality || sound ? box : null, chosen, chips: [quality, sound].filter(Boolean) };
}

// A video's preview beside its name (§15), so several found on one page can be told apart: one picture and its
// length. A stream's picture comes from hdm (its smallest quality's first piece, three asked at a time); a whole
// video file's the browser reads itself. Without one, the box stays empty.
let previewsRunning = 0;
const previewsWaiting = [];

function thumbnail(tab, item, streamed) {
  const box = document.createElement("span");
  box.className = "thumb";
  box.setAttribute("aria-hidden", "true");
  const time = document.createElement("span");
  time.className = "time";
  // Its length over the picture, after a play mark, as players show it.
  const overlay = document.createElement("span");
  overlay.className = "overlay";
  const play = document.createElement("span");
  play.className = "play";
  play.append(icon("play"));
  overlay.append(play, time);
  const show = (src, seconds) => {
    const video = document.createElement("video");
    video.muted = true;
    video.preload = "metadata";
    video.playsInline = true;
    video.onloadedmetadata = () => {
      if (!seconds && Number.isFinite(video.duration)) time.textContent = length(video.duration);
    };
    video.onerror = () => video.remove();
    video.src = src;
    box.prepend(video);
    if (seconds) time.textContent = length(seconds);
  };
  box.append(overlay);
  if (!streamed) {
    // Asked for as its page would (cookies, Referer): TikTok's server refuses its files to anyone without its cookie.
    ask({ type: "media-access", url: item.url, page: tab.url }).catch(() => null).then(() => show(`${item.url}#t=1`));
    return box;
  }
  if (item.seconds) time.textContent = length(item.seconds);
  const run = async () => {
    previewsRunning += 1;
    const answer = await ask({ type: "preview", tabId: tab.id, item }).catch(() => null);
    previewsRunning -= 1;
    if (previewsWaiting.length) previewsWaiting.shift()();
    if (!(answer && answer.ok && answer.preview)) return;
    const bytes = Uint8Array.from(atob(answer.preview.mp4), (c) => c.charCodeAt(0));
    show(URL.createObjectURL(new Blob([bytes], { type: "video/mp4" })), answer.preview.seconds || item.seconds);
  };
  if (previewsRunning < 3) run();
  else previewsWaiting.push(run);
  return box;
}

// One item as a card (§13): its picture with its length, a badge for its kind, its name, what it is (size, site) or
// how adding it went; then ✏ (name it first), its quality and sound, and Download with ▾ for the other ways (Add to
// Queue, Sound Only, Copy Address). × takes it off the list.
function row(tab, item, label, facts) {
  const li = document.createElement("li");
  li.className = "card";
  const pick = item.kind === "stream" ? pickers(item) : { chosen: () => ({}), chips: [] };
  const streamed = ["stream", "dash", "page-dash"].includes(item.kind);
  // The picture: videos and streams get a preview; a sound or a file, its kind's mark.
  let thumb;
  if (streamed || item.kind === "video") {
    thumb = thumbnail(tab, item, streamed);
  } else {
    thumb = document.createElement("span");
    thumb.className = "thumb sound";
    thumb.setAttribute("aria-hidden", "true");
    thumb.append(icon(item.kind === "audio" ? "sound" : "file"));
  }
  const body = document.createElement("div");
  body.className = "body";
  const name = document.createElement("span");
  name.className = "title";
  const tag = document.createElement("span");
  tag.className = "badge";
  tag.textContent = badge(item, label);
  name.append(tag, document.createTextNode(label));
  name.title = item.url || label;
  const detail = document.createElement("span");
  detail.className = "detail";
  const host = (() => { try { return item.url ? new URL(item.url).hostname : ""; } catch { return ""; } })();
  detail.textContent = [...facts, host].filter(Boolean).join(" · ");
  const tools = document.createElement("div");
  tools.className = "tools";
  body.append(name, detail, tools);
  // Its checkbox, over the picture's corner, for Select All and the selection's Download.
  const tick = document.createElement("label");
  tick.className = "pick";
  tick.title = "Select this item";
  const box = document.createElement("input");
  box.type = "checkbox";
  box.setAttribute("aria-label", `Select ${label}`);
  box.onchange = selectionChanged;
  tick.append(box);
  li.append(tick, thumb, body);
  // Everything that adds it, disabled while it is being added.
  const controls = [];
  const enable = (on) => controls.forEach((c) => { c.disabled = !on; });
  // Adds it (`extra`: start at once, sound only, a name), and says how that went in the card's second line.
  const add = async (extra) => {
    enable(false);
    detail.className = "detail";
    detail.textContent = "Adding…";
    // Download and Add are a request for the app: it's opened in the background when it isn't running (§13).
    const answer = await ask({ type: "add", tabId: tab.id, item, ...pick.chosen(), ...extra, open: true });
    if (answer && answer.ok) {
      detail.className = "detail ok";
      detail.textContent = extra.start ? "Downloading" : "Added to the queue";
      appAnswered();
    } else if (answer && answer.not_running) {
      // The app isn't open: offer to open it, then download this at once.
      detail.className = "detail";
      detail.textContent = "Herb's Download Manager isn't running.";
      body.append(openButton("Open and Download", async () => {
        detail.textContent = "Opening Herb's Download Manager…";
        const opened = await ask({ type: "open" });
        if (!(opened && opened.ok)) return opened;
        detail.textContent = "Adding…";
        const added = await ask({ type: "add", tabId: tab.id, item, ...pick.chosen(), ...extra, start: true });
        if (added && added.ok) {
          detail.className = "detail ok";
          detail.textContent = "Downloading";
          appAnswered();
        }
        return added;
      }, detail));
    } else {
      detail.className = "detail bad";
      detail.textContent = (answer && answer.error) || "Herb's Download Manager didn't answer.";
      enable(true);
    }
  };
  // The selection's Download and Add to Queue: added as it is (its site's rule still names it).
  li.addNow = (extra) => {
    if (!main.disabled) add(extra);
  };
  // A name first: the one it would get, to change, then Download or Add.
  const nameFirst = async (start) => {
    if (body.querySelector(".naming")) return;
    const { name: proposed } = await ask({ type: "name", tabId: tab.id, item });
    const form = document.createElement("form");
    form.className = "naming";
    const field = document.createElement("input");
    field.value = proposed || "";
    field.setAttribute("aria-label", "Name to save it under");
    const ok = document.createElement("button");
    ok.type = "submit";
    ok.textContent = start ? "Download" : "Add";
    const cancel = document.createElement("button");
    cancel.type = "button";
    cancel.textContent = "Cancel";
    cancel.onclick = () => form.remove();
    form.append(field, ok, cancel);
    form.onsubmit = (e) => {
      e.preventDefault();
      form.remove();
      add({ ...(start ? { start: true } : {}), name: field.value.trim() || proposed });
    };
    tools.before(form);
    field.focus();
    field.setSelectionRange(0, Math.max(0, field.value.lastIndexOf(".")));
  };
  // Download Section: From and To (H:MM:SS), To filled in with its length when known; then Download.
  const section = () => {
    if (body.querySelector(".section")) return;
    const form = document.createElement("form");
    form.className = "naming section";
    const time = (label, value) => {
      const field = document.createElement("input");
      field.className = "time-field";
      field.value = value;
      field.placeholder = "0:00:00";
      field.inputMode = "numeric";
      field.setAttribute("aria-label", label);
      field.title = `${label} (hours, minutes and seconds: 1:02:30)`;
      const words = document.createElement("label");
      words.className = "time-label";
      words.textContent = label;
      words.append(field);
      form.append(words);
      return field;
    };
    const clock = (s) => { const t = Math.round(s); return `${Math.floor(t / 3600)}:${String(Math.floor(t / 60) % 60).padStart(2, "0")}:${String(t % 60).padStart(2, "0")}`; };
    const from = time("From", "0:00:00");
    const to = time("To", item.seconds ? clock(item.seconds) : "");
    const ok = document.createElement("button");
    ok.type = "submit";
    ok.textContent = "Download";
    const cancel = document.createElement("button");
    cancel.type = "button";
    cancel.textContent = "Cancel";
    cancel.onclick = () => form.remove();
    form.append(ok, cancel);
    const valid = (t) => /^\d{1,6}(:[0-5]?\d){0,2}$/.test(t.trim());
    form.onsubmit = (e) => {
      e.preventDefault();
      const [a, b] = [from.value.trim(), to.value.trim()];
      const bad = [from, to].find((f) => f.value.trim() && !valid(f.value));
      if (bad) {
        detail.className = "detail bad";
        detail.textContent = `“${bad.value}” isn't a time: give hours, minutes and seconds, such as 1:02:30`;
        bad.focus();
        return;
      }
      form.remove();
      add({ start: true, from: a || "0:00:00", ...(b ? { to: b } : {}) });
    };
    tools.before(form);
    from.focus();
    from.select();
  };
  // What Download does on this site (its naming rule's action, §13): download it, ask for a name first, take only
  // the sound, or copy the address. `start`: Download now; otherwise it waits its turn in the queue.
  const action = siteAction === "sound" && !streamed ? "download" : siteAction === "copy" && !item.url ? "download" : siteAction;
  const choose = async (start) => {
    const when = start ? { start: true } : {};
    if (action === "sound") return add({ ...when, soundOnly: true });
    if (action === "copy") return copy();
    if (action === "ask") return nameFirst(start);
    return add(when);
  };
  const copy = async () => {
    await navigator.clipboard.writeText(item.url).catch(() => {});
    detail.className = "detail ok";
    detail.textContent = "Address copied";
  };
  // ✏ Name it first.
  const pencil = document.createElement("button");
  pencil.className = "tool";
  pencil.append(icon("pencil"));
  pencil.title = "Name it first: change the name it's saved under, then download it";
  pencil.setAttribute("aria-label", `Name ${label} first, then download it`);
  pencil.onclick = () => nameFirst(true);
  controls.push(pencil);
  tools.append(pencil);
  // Its quality and sound (a stream that offers a choice), or what it is: format and quality or size.
  if (pick.chips.length) {
    for (const chip of pick.chips) {
      controls.push(chip);
      tools.append(chip);
    }
  } else {
    const what = document.createElement("span");
    what.className = "chip static";
    const format = document.createElement("span");
    format.className = "format";
    format.textContent = streamed ? "MP4" : badge(item, label);
    what.append(format, document.createTextNode(item.quality || size(item.size) || length(item.seconds) || ""));
    tools.append(what);
  }
  // Download, with ▾ for the other ways.
  const split = document.createElement("span");
  split.className = "split";
  const main = document.createElement("button");
  main.className = "main";
  main.append(icon("down"), document.createTextNode(action === "copy" ? "Copy Address" : "Download"));
  main.title = {
    download: "Download it now",
    ask: "Name it, then download it now",
    sound: "Download only its sound now (.m4a, or .mp3), as the site sends it",
    copy: "Copy its address",
  }[action] || "Download it now";
  main.setAttribute("aria-label", `${main.title}: ${label}`);
  main.onclick = () => choose(true);
  const more = document.createElement("button");
  more.className = "more";
  more.append(icon("chevron"));
  more.title = "More ways: Add to Queue, Sound Only, Copy Address";
  more.setAttribute("aria-label", `More ways to add ${label}`);
  more.setAttribute("aria-haspopup", "menu");
  more.setAttribute("aria-expanded", "false");
  const menu = document.createElement("div");
  menu.className = "menu";
  menu.setAttribute("role", "menu");
  menu.hidden = true;
  const entry = (text, help, run) => {
    const b = document.createElement("button");
    b.textContent = text;
    b.title = help;
    b.setAttribute("role", "menuitem");
    b.onclick = () => {
      close();
      run();
    };
    menu.append(b);
  };
  entry("Add to Queue", "Add it to the queue, to download in its turn", () => choose(false));
  if (action !== "download") entry("Download Now", "Download it now, as it is", () => add({ start: true }));
  // A streamed video's sound alone, as the site sends it (never re-encoded); a page's video with no sound gets none.
  if (streamed && !(item.kind === "page-dash" && item.sound === false)) {
    entry("Sound Only", "Download only its sound (.m4a, or .mp3), as the site sends it", () => add({ start: true, soundOnly: true }));
  }
  // A part of a long stream: from and to (HLS streams, whose pieces say their length).
  if (item.kind === "stream") entry("Download Section…", "Download only a part of it: from one time to another (hours, minutes, seconds)", () => section());
  if (item.url && action !== "copy") entry("Copy Address", "Copy its address", copy);
  const close = () => {
    menu.hidden = true;
    more.setAttribute("aria-expanded", "false");
    document.removeEventListener("click", outside, true);
  };
  const outside = (e) => { if (!split.contains(e.target)) close(); };
  more.onclick = () => {
    if (!menu.hidden) return close();
    menu.hidden = false;
    more.setAttribute("aria-expanded", "true");
    document.addEventListener("click", outside, true);
    menu.firstChild.focus();
  };
  // The keyboard: ↑ ↓ (and Tab, ⇧Tab) move through the items, wrapping round, Home and End go to the ends, Return
  // runs the item, Esc closes the menu.
  menu.onkeydown = (e) => {
    const items = [...menu.querySelectorAll("button")];
    const at = items.indexOf(document.activeElement);
    const go = (i) => { e.preventDefault(); items[(i + items.length) % items.length].focus(); };
    if (e.key === "Escape") {
      e.preventDefault();
      close();
      more.focus();
    } else if (e.key === "ArrowDown" || (e.key === "Tab" && !e.shiftKey)) go(at + 1);
    else if (e.key === "ArrowUp" || (e.key === "Tab" && e.shiftKey)) go(at - 1);
    else if (e.key === "Home") go(0);
    else if (e.key === "End") go(items.length - 1);
  };
  // ↓ on ▾ opens the menu, as on a menu button.
  more.onkeydown = (e) => {
    if (e.key === "ArrowDown" && menu.hidden) {
      e.preventDefault();
      more.click();
    }
  };
  split.append(main, more, menu);
  controls.push(main, more);
  tools.append(split);
  // ×: off the list (until the tab goes to another page).
  const remove = document.createElement("button");
  remove.className = "remove";
  remove.append(icon("close"));
  remove.title = "Remove from this list";
  remove.setAttribute("aria-label", `Remove ${label} from this list`);
  remove.onclick = async () => {
    li.remove();
    selectionChanged();
    if (item.found) gone.push(item.url);
    await ask({ type: "remove", tabId: tab.id, item: { kind: item.kind, url: item.url, manifest: item.manifest } });
    if (!item.found && !document.querySelector("#items li")) $("empty").hidden = false;
  };
  li.append(remove);
  return li;
}

// A button under an item or a note that opens the app and carries on (`then`): it goes once done, and a failure is
// written into `detail`.
// The app answered after all (opened from the list): a scan that found it closed looks again, so "isn't running"
// doesn't stay under a download it just took.
function appAnswered() {
  const note = $("scan-note");
  if (note.hidden || !/isn't running/.test(note.textContent)) return;
  note.hidden = true;
  note.parentElement.querySelectorAll(":scope > button.open").forEach((b) => b.remove());
  $("scan").click();
}

function openButton(label, then, detail) {
  const open = document.createElement("button");
  open.className = "text-button open";
  open.textContent = label;
  open.onclick = async () => {
    open.disabled = true;
    const answer = await then();
    if (answer && answer.ok) {
      open.remove();
    } else {
      detail.className = detail.className.replace(/\b(ok)\b/, "") + " bad";
      detail.textContent = (answer && answer.error) || "Herb's Download Manager didn't answer.";
      open.disabled = false;
    }
  };
  return open;
}

// The extension's version beside Naming Rules ("Version 1.17").
const version = api.runtime.getManifest && api.runtime.getManifest().version;
$("version").textContent = version ? `Version ${version}` : "";
show();
// What the tab finds while the list is open (after Look Again, say) joins it; in the side panel, so does another tab.
api.storage.session.onChanged.addListener(() => show());
if (inPanel) {
  api.tabs.onActivated.addListener(() => show());
  api.tabs.onUpdated.addListener((id, change) => {
    if (change.status === "complete" || change.title) show();
  });
}
