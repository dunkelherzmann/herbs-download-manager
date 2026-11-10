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

function title(item) {
  const last = decodeURIComponent(item.url.split(/[?#]/)[0].split("/").pop() || "");
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

async function show() {
  const tab = await currentTab();
  const state = await ask({ type: "state", tabId: tab.id });
  $("on").checked = state.on;
  $("on").onchange = async () => {
    $("on").checked = (await ask({ type: "toggle", tabId: tab.id })).on;
  };
  const list = $("items");
  list.replaceChildren();
  $("empty").hidden = state.items.length > 0;
  for (const item of state.items) {
    const streamed = ["stream", "dash", "page-dash"].includes(item.kind);
    const facts = item.kind === "page-dash"
      ? [item.sound ? "Video with sound" : "Video", item.quality, length(item.seconds)]
      : [KIND[item.kind], size(item.size)];
    list.append(row(tab, item, streamed ? tab.title || title(item) : title(item), facts));
  }
  $("scan").onclick = async () => {
    $("scan").disabled = true;
    $("scan").textContent = "Looking through the page…";
    const note = $("scan-note");
    note.hidden = true;
    note.parentElement.querySelectorAll(":scope > button.open").forEach((b) => b.remove());
    const answer = await ask({ type: "scan", tabId: tab.id });
    $("scan").disabled = false;
    $("scan").textContent = "Find Files on This Page";
    const found = $("found");
    found.replaceChildren();
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
    for (const f of files) {
      const item = { url: f.url, name: f.name, size: f.size || 0, kind: f.stream ? "stream" : "file", stream: f.stream };
      found.append(row(tab, item, f.name, [f.kind, size(f.size)]));
    }
  };
}

// One item: its name, what it is (kind, size, site), and a click that adds it to the queue.
function row(tab, item, label, facts) {
  const li = document.createElement("li");
  const button = document.createElement("button");
  button.className = "item";
  const name = document.createElement("span");
  name.className = "name";
  name.textContent = label;
  const detail = document.createElement("span");
  detail.className = "detail";
  const host = (() => { try { return item.url ? new URL(item.url).hostname : ""; } catch { return ""; } })();
  detail.textContent = [...facts, host].filter(Boolean).join(" · ");
  button.append(name, detail);
  button.title = item.url || label;
  {
    button.setAttribute("aria-label", `Add ${label} to the queue`);
    button.onclick = async () => {
      button.disabled = true;
      detail.className = "detail";
      detail.textContent = "Adding…";
      const answer = await ask({ type: "add", tabId: tab.id, item });
      if (answer && answer.ok) {
        detail.className = "detail ok";
        detail.textContent = "Added to the queue";
      } else if (answer && answer.not_running) {
        // The app isn't open: offer to open it, then download this at once.
        detail.className = "detail";
        detail.textContent = "Herb's Download Manager isn't running.";
        li.append(openButton("Open and Download", async () => {
          detail.textContent = "Opening Herb's Download Manager…";
          const opened = await ask({ type: "open" });
          if (!(opened && opened.ok)) return opened;
          detail.textContent = "Adding…";
          const added = await ask({ type: "add", tabId: tab.id, item, start: true });
          if (added && added.ok) {
            detail.className = "detail ok";
            detail.textContent = "Downloading";
          }
          return added;
        }, detail));
      } else {
        detail.className = "detail bad";
        detail.textContent = (answer && answer.error) || "Herb's Download Manager didn't answer.";
        button.disabled = false;
      }
    };
  }
  li.append(button);
  return li;
}

// A button under an item or a note that opens the app and carries on (`then`): it goes once done, and a failure is
// written into `detail`.
function openButton(label, then, detail) {
  const open = document.createElement("button");
  open.className = "scan open";
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

show();
