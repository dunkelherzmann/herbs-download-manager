// Herb's Download Manager for Chrome, Edge, Brave, Vivaldi and Firefox (SPECIFICATION.md §13): hands each new
// download to the app through native messaging (the app's `hdm`), and the browser keeps any download the app
// doesn't take: when the app isn't open, the link doesn't work for it, or the window is private.

const api = globalThis.browser ?? chrome;
const HOST = "org.herbsdownloadmanager.hdm";
const NAME = "Herb's Download Manager";

async function isOn() {
  const { on } = await api.storage.local.get({ on: true });
  return on;
}

async function showState() {
  const on = await isOn();
  await api.action.setBadgeText({ text: on ? "" : "off" });
  await api.action.setTitle({ title: on ? `${NAME}: on (click to turn off)` : `${NAME}: off (click to turn on)` });
}

// A short sign on the button when the app didn't take a download, with the reason in its hover text.
async function tell(reason) {
  await api.action.setBadgeText({ text: "!" });
  await api.action.setTitle({ title: `${NAME}: the browser kept the last download. ${reason}` });
  setTimeout(showState, 15000);
}

const web = (url) => /^https?:/i.test(url || "");

// The cookies the browser would send with the link, as a Cookie header.
async function cookiesFor(url) {
  try {
    const all = await api.cookies.getAll({ url });
    return all.map((c) => `${c.name}=${c.value}`).join("; ");
  } catch {
    return "";
  }
}

// Asks the app to download the link; resolves to its answer ({ok, error, message}).
async function send(url, referer) {
  const message = { command: "add", urls: [url], referer: referer || "", cookies: await cookiesFor(url), check: true };
  try {
    return await api.runtime.sendNativeMessage(HOST, message);
  } catch {
    return { ok: false, error: `${NAME} isn't set up for this browser, or isn't installed.` };
  }
}

api.downloads.onCreated.addListener(async (item) => {
  const url = item.finalUrl || item.url;
  if (!(await isOn()) || item.incognito || item.state !== "in_progress" || !web(url)) return;
  // Paused while the app looks at the link, so neither side downloads it twice. One that finished before it could
  // pause (a small file on a fast line) stays with the browser.
  await api.downloads.pause(item.id).catch(() => {});
  const [paused] = await api.downloads.search({ id: item.id });
  // (Firefox reports a paused download as "interrupted".)
  if (!paused || paused.state === "complete") return;
  const answer = await send(url, item.referrer);
  const [now] = await api.downloads.search({ id: item.id });
  if (answer && answer.ok) {
    await api.downloads.cancel(item.id).catch(() => {});
    await api.downloads.erase({ id: item.id }).catch(() => {});
  } else {
    if (now && now.state !== "complete") await api.downloads.resume(item.id).catch(() => {});
    await tell((answer && answer.error) || `${NAME} didn't answer.`);
  }
});

// Right-click a link: Download with Herb's Download Manager.
async function makeMenu() {
  await api.contextMenus.removeAll();
  api.contextMenus.create({ id: "hdm-link", title: `Download with ${NAME}`, contexts: ["link", "image", "video", "audio"] });
}
api.runtime.onInstalled.addListener(() => { makeMenu(); showState(); });
api.runtime.onStartup.addListener(() => { makeMenu(); showState(); });

api.contextMenus.onClicked.addListener(async (info) => {
  const url = info.linkUrl || info.srcUrl;
  if (!web(url)) return;
  const answer = await send(url, info.pageUrl);
  if (!(answer && answer.ok)) await tell((answer && answer.error) || `${NAME} didn't answer.`);
});

// The toolbar button turns the hand-over on and off.
api.action.onClicked.addListener(async () => {
  await api.storage.local.set({ on: !(await isOn()) });
  await showState();
});
