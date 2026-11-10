// The toolbar button's list (SPECIFICATION.md §13, §15): whether the browser's downloads go to the app, and the
// videos and sounds this tab played, each added to the app's queue with a click. background.js does the work.

const api = globalThis.browser ?? chrome;
const $ = (id) => document.getElementById(id);
const ask = (message) => api.runtime.sendMessage(message);

const KIND = { stream: "Video stream", dash: "Video stream (DASH)", video: "Video", audio: "Sound" };

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
    const li = document.createElement("li");
    const button = document.createElement("button");
    button.className = "item";
    const name = document.createElement("span");
    name.className = "name";
    name.textContent = item.kind === "stream" ? tab.title || title(item) : title(item);
    const detail = document.createElement("span");
    detail.className = "detail";
    const host = (() => { try { return new URL(item.url).hostname; } catch { return ""; } })();
    detail.textContent = [KIND[item.kind], size(item.size), host].filter(Boolean).join(" · ");
    button.append(name, detail);
    button.title = item.url;
    if (item.kind === "dash") {
      button.disabled = true;
      detail.textContent += " · not supported yet";
    } else {
      button.setAttribute("aria-label", `Add ${name.textContent} to the queue`);
      button.onclick = async () => {
        button.disabled = true;
        detail.className = "detail";
        detail.textContent = "Adding…";
        const answer = await ask({ type: "add", tabId: tab.id, item });
        if (answer && answer.ok) {
          detail.className = "detail ok";
          detail.textContent = "Added to the queue";
        } else {
          detail.className = "detail bad";
          detail.textContent = (answer && answer.error) || "Herb's Download Manager didn't answer.";
          button.disabled = false;
        }
      };
    }
    li.append(button);
    list.append(li);
  }
}

show();
