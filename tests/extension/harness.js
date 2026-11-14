// Runs the browser extension's background script (extension/background.js, with naming.js) in Node with a stand-in
// for the browser's extension API, so its detection, ordering, naming and hand-over can be tested without a browser.
// Each `load()` is a fresh background: its storage, tabs, network answers and what it sent to `hdm` are its own.

const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const EXT = path.join(__dirname, "..", "..", "extension");

// A listener list: `addListener` keeps them, `fire` calls them all and gives back their results.
function event() {
  const fns = [];
  return { addListener: (f) => fns.push(f), removeListener: (f) => fns.splice(fns.indexOf(f), 1), fire: (...a) => fns.map((f) => f(...a)), fns };
}

// Storage with get(defaults | key | keys), set, remove.
function area() {
  const data = {};
  return {
    data,
    onChanged: event(),
    async get(keys) {
      if (keys == null) return { ...data };
      if (typeof keys === "string") return keys in data ? { [keys]: data[keys] } : {};
      if (Array.isArray(keys)) return Object.fromEntries(keys.filter((k) => k in data).map((k) => [k, data[k]]));
      return Object.fromEntries(Object.entries(keys).map(([k, d]) => [k, k in data ? structuredClone(data[k]) : d]));
    },
    async set(o) {
      Object.assign(data, structuredClone(o));
    },
    async remove(k) {
      for (const key of [].concat(k)) delete data[key];
    },
  };
}

function load({ tabs = [], fetch: answer = () => ({ status: 404, body: "" }), native = () => ({ ok: true }), script = () => "", cookies = [] } = {}) {
  const sent = [];
  // Each setUiOptions call's `enabled`: whether the browser's own downloads panel shows.
  const ui = [];
  const fetches = [];
  const rules = { added: [], removed: [] };
  const menus = new Map();
  const tabMap = new Map(tabs.map((t) => [t.id, t]));
  const chrome = {
    action: { setBadgeText: async () => {}, setBadgeBackgroundColor: async () => {}, setPopup: async () => {}, setTitle: async () => {} },
    contextMenus: {
      create: (item) => menus.set(item.id, item),
      remove: (id, done) => {
        for (const [k, v] of menus) if (k === id || v.parentId === id) menus.delete(k);
        if (done) done();
      },
      removeAll: async () => menus.clear(),
      onClicked: event(),
    },
    cookies: { getAll: async () => cookies },
    downloads: { onCreated: event(), onChanged: event(), pause: async () => {}, resume: async () => {}, cancel: async () => {}, erase: async () => {},
                 search: async () => [], setUiOptions: async (o) => { ui.push(o.enabled); } },
    runtime: {
      lastError: undefined,
      onInstalled: event(),
      onStartup: event(),
      onMessage: event(),
      openOptionsPage: async () => {},
      sendNativeMessage: async (host, message) => {
        sent.push(message);
        return native(message);
      },
    },
    scripting: { executeScript: async ({ args }) => [{ result: script(...(args || [])) }] },
    sidePanel: { setPanelBehavior: async () => {} },
    storage: { local: area(), session: area() },
    tabs: {
      get: async (id) => {
        if (!tabMap.has(id)) throw new Error("no tab");
        return tabMap.get(id);
      },
      query: async () => [...tabMap.values()].filter((t) => t.active),
      reload: async () => {},
      onActivated: event(),
      onRemoved: event(),
      onUpdated: event(),
    },
    webRequest: { onResponseStarted: event(), onSendHeaders: event(), OnSendHeadersOptions: { EXTRA_HEADERS: "extraHeaders" } },
    declarativeNetRequest: {
      updateSessionRules: async ({ addRules = [], removeRuleIds = [] }) => {
        rules.added.push(...addRules);
        rules.removed.push(...removeRuleIds);
      },
    },
  };
  const fakeFetch = async (url, options = {}) => {
    fetches.push({ url, options });
    const { status = 200, body = "" } = answer(url, options) || {};
    const bytes = typeof body === "string" ? new TextEncoder().encode(body) : body;
    return { ok: status >= 200 && status < 300, status, text: async () => body, json: async () => JSON.parse(body),
             arrayBuffer: async () => bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) };
  };
  const context = vm.createContext({
    chrome, console, URL, URLSearchParams, AbortSignal, Uint8Array, TextEncoder, TextDecoder, setTimeout, clearTimeout, structuredClone,
    navigator: { clipboard: { writeText: async () => {} } },
    fetch: fakeFetch,
    importScripts: (...files) => files.forEach((f) => vm.runInContext(fs.readFileSync(path.join(EXT, f), "utf8"), context, { filename: f })),
  });
  vm.runInContext(fs.readFileSync(path.join(EXT, "background.js"), "utf8"), context, { filename: "background.js" });
  // The background script's own names (functions and top-level constants), by name.
  const get = (name) => vm.runInContext(name, context);
  // A message from the list, as the popup sends it, and the background's answer.
  const ask = (message, sender = {}) => new Promise((reply) => chrome.runtime.onMessage.fire(message, sender, reply));
  // A response the browser saw a tab get, then the background's notes about it settled.
  const respond = async (d) => {
    chrome.webRequest.onResponseStarted.fire({ tabId: 1, statusCode: 200, type: "xmlhttprequest", responseHeaders: [], ...d });
    await get("noting");
  };
  const items = async (tabId = 1) => (await chrome.storage.session.get({ [`media-${tabId}`]: [] }))[`media-${tabId}`];
  return { chrome, context, get, ask, respond, items, sent, fetches, rules, menus, ui };
}

// Response headers as the browser gives them.
const headers = (o) => Object.entries(o).map(([name, value]) => ({ name, value }));

module.exports = { load, headers };
