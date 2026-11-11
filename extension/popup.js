// The toolbar button's list (SPECIFICATION.md §13, §15): whether the browser's downloads go to the app, and the
// videos and sounds this tab played, each added to the app's queue with a click. background.js does the work.

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

// A short word in a button's hover text (and for screen readers), back to its own after 3 s.
function flash(button, words) {
  const own = button.title;
  button.title = words;
  button.setAttribute("aria-description", words);
  setTimeout(() => {
    button.title = own;
    button.removeAttribute("aria-description");
  }, 3000);
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
  const state = await ask({ type: "state", tabId: tab.id });
  $("on").checked = state.on;
  $("on").onchange = async () => {
    $("on").checked = (await ask({ type: "toggle", tabId: tab.id })).on;
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
  // Refresh: what the page plays now goes first, marked "Playing now".
  $("refresh").onclick = async () => {
    busy($("refresh"), true);
    const answer = await ask({ type: "refresh", tabId: tab.id }).catch(() => null);
    busy($("refresh"), false);
    if (answer && answer.ok && !answer.playing) flash($("refresh"), "Nothing playing now");
    shownKey = "";
    show();
  };
  // Look Again (and "No video?"): the page loads again, so its player fetches its video again.
  const lookAgain = async (b, label) => {
    b.disabled = true;
    b.textContent = "Loading the page again…";
    await ask({ type: "look-again", tabId: tab.id });
    setTimeout(() => {
      b.disabled = false;
      b.textContent = label;
    }, 3000);
  };
  $("again").onclick = () => lookAgain($("again"), "Look Again");
  $("no-video").onclick = () => lookAgain($("no-video"), "No video?");
  const key = `${tab.id}|${state.order}|${siteAction}|${state.items.map((i) => `${i.playing ? "*" : ""}${i.url || i.manifest.length}`).join("|")}`;
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
    if (item.playing) facts.unshift("Playing now");
    list.append(row(tab, item, streamed ? tab.title || title(item, tab) : title(item, tab), facts));
  }
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
    const files = answer.found || [];
    if (!files.length) {
      note.className = "note";
      note.textContent = "No files or media found on this page.";
      note.hidden = false;
    }
    $("found-heading").hidden = !files.length;
    for (const f of files) {
      const item = { url: f.url, name: f.name, size: f.size || 0, kind: f.stream ? "stream" : "file", stream: f.stream, found: f.kind };
      found.append(row(tab, item, f.name, [f.kind, size(f.size)]));
    }
  };
  // The page is looked through as soon as the list opens (and in the side panel, whenever its tab shows another
  // page), so its files and media are found without playing anything first.
  if (tab.url !== scannedUrl && /^https?:/.test(tab.url || "")) {
    scannedUrl = tab.url;
    $("scan").click();
  }
}
let scannedUrl = "";

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
  li.append(thumb, body);
  // Everything that adds it, disabled while it is being added.
  const controls = [];
  const enable = (on) => controls.forEach((c) => { c.disabled = !on; });
  // Adds it (`extra`: start at once, sound only, a name), and says how that went in the card's second line.
  const add = async (extra) => {
    enable(false);
    detail.className = "detail";
    detail.textContent = "Adding…";
    const answer = await ask({ type: "add", tabId: tab.id, item, ...pick.chosen(), ...extra });
    if (answer && answer.ok) {
      detail.className = "detail ok";
      detail.textContent = extra.start ? "Downloading" : "Added to the queue";
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
        }
        return added;
      }, detail));
    } else {
      detail.className = "detail bad";
      detail.textContent = (answer && answer.error) || "Herb's Download Manager didn't answer.";
      enable(true);
    }
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
  menu.onkeydown = (e) => { if (e.key === "Escape") { close(); more.focus(); } };
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
    if (item.found) return;
    await ask({ type: "remove", tabId: tab.id, item: { kind: item.kind, url: item.url, manifest: item.manifest } });
    if (!document.querySelector("#items li")) $("empty").hidden = false;
  };
  li.append(remove);
  return li;
}

// A button under an item or a note that opens the app and carries on (`then`): it goes once done, and a failure is
// written into `detail`.
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
