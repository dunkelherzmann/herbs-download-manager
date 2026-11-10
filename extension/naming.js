// Naming rules (SPECIFICATION.md §13): per site, a name pattern, find-and-replace pairs, a length limit, a
// sub-folder and what a click in the list does. Used by background.js, popup.js and options.js.

// A rule: { site, pattern, selector, replace: [[find, with]…], max, folder, action }.
// action: "download" (add to the queue), "ask" (a name first), "sound" (sound only), "copy" (the address).
const ACTIONS = [
  ["download", "Download"],
  ["ask", "Ask for a name first"],
  ["sound", "Sound only"],
  ["copy", "Copy the address"],
];

// A site's host matches a rule's site: the site itself or any of its subdomains (youtube.com covers m.youtube.com).
function ruleFor(rules, pageUrl) {
  let host = "";
  try {
    host = new URL(pageUrl).hostname.toLowerCase();
  } catch {
    return null;
  }
  const covers = (site) => {
    const s = site.trim().toLowerCase().replace(/^\*\./, "").replace(/^www\./, "");
    return s && (host === s || host.endsWith(`.${s}`));
  };
  // The most particular site wins (video.example.com over example.com).
  return (rules || []).filter((r) => covers(r.site || "")).sort((a, b) => b.site.length - a.site.length)[0] || null;
}

// The site's name without its top-level domain or "www": www.youtube.com → youtube, bbc.co.uk → bbc.
function siteName(pageUrl) {
  try {
    const parts = new URL(pageUrl).hostname.replace(/^www\./, "").split(".");
    const short = ["co", "com", "org", "net", "ac", "gov", "edu"];
    if (parts.length > 2 && short.includes(parts[parts.length - 2])) return parts[parts.length - 3];
    return parts.length > 1 ? parts[parts.length - 2] : parts[0];
  } catch {
    return "";
  }
}

// A name a file can have: letters, numbers and marks kept, characters files can't hold and line breaks taken out,
// spaces gathered, no leading dots; "no-name" when nothing is left.
function cleanName(text) {
  const out = (text || "")
    .normalize("NFC")
    .replace(/[\\/:*?"<>|\u0000-\u001f\u007f]/g, " ")
    .replace(/\s+/g, " ")
    .replace(/^[\s.]+/, "")
    .trim();
  return out || "no-name";
}

// The name a rule gives (without the extension): its pattern filled in, its pairs replaced in order, cut to its
// length. `page`: { title, url, selected } (selected: the text its selector found on the page).
function nameByRule(rule, page) {
  let name = (rule.pattern || "%title")
    .replace(/%title/g, page.title || "")
    .replace(/%site/g, siteName(page.url))
    .replace(/%selector/g, page.selected || "");
  for (const [find, put] of rule.replace || []) {
    if (find) name = name.split(find).join(put || "");
  }
  name = cleanName(name);
  const max = Number(rule.max) || 0;
  if (max > 0 && name.length > max) name = name.slice(0, max).trim() || name.slice(0, max);
  return name;
}

// A sub-folder as the app takes it: parts separated by "/", each cleaned, ".." and empty parts left out.
function cleanFolder(text) {
  return (text || "")
    .split(/[\\/]+/)
    .map((p) => p.trim())
    .filter((p) => p && p !== "." && p !== "..")
    .map(cleanName)
    .join("/");
}

if (typeof module !== "undefined") module.exports = { ACTIONS, ruleFor, siteName, cleanName, nameByRule, cleanFolder };
