// The Naming Rules page (SPECIFICATION.md §13): rules are kept in the browser's storage, saved as they're typed.

const api = globalThis.browser ?? chrome;
const box = document.getElementById("rules");

// "find => with" lines ↔ pairs.
const pairsOf = (text) => text.split("\n").map((l) => l.split("=>")).filter((p) => p[0].trim()).map(([f, ...w]) => [f.trim(), w.join("=>").trim()]);
const textOf = (pairs) => (pairs || []).map(([f, w]) => `${f} => ${w}`).join("\n");

function read() {
  return [...box.querySelectorAll(".rule")].map((f) => ({
    site: f.elements.site.value.trim(),
    pattern: f.elements.pattern.value.trim() || "%title",
    selector: f.elements.selector.value.trim(),
    replace: pairsOf(f.elements.replace.value),
    max: Number(f.elements.max.value) || 0,
    folder: cleanFolder(f.elements.folder.value),
    action: f.elements.action.value,
  }));
}

async function save() {
  await api.storage.local.set({ rules: read().filter((r) => r.site) });
}

// What the rule makes of an example page on its site.
function example(f) {
  const rule = read()[[...box.querySelectorAll(".rule")].indexOf(f)];
  const site = rule.site || "example.com";
  const name = nameByRule(rule, { title: "A Page's Title", url: `https://www.${site}/watch`, selected: rule.selector ? "Text from the page" : "" });
  f.querySelector(".example").textContent = `Example: ${rule.folder ? `${rule.folder}/` : ""}${name}.mp4`;
}

function add(rule) {
  const f = document.getElementById("rule").content.firstElementChild.cloneNode(true);
  for (const [value, label] of ACTIONS) f.elements.action.append(new Option(label, value));
  f.elements.site.value = rule.site || "";
  f.elements.pattern.value = rule.pattern && rule.pattern !== "%title" ? rule.pattern : "";
  f.elements.selector.value = rule.selector || "";
  f.elements.replace.value = textOf(rule.replace);
  f.elements.max.value = rule.max || "";
  f.elements.folder.value = rule.folder || "";
  f.elements.action.value = rule.action || "download";
  f.addEventListener("input", () => { example(f); save(); });
  f.querySelector(".remove").onclick = () => { f.remove(); save(); shown(); };
  box.append(f);
  example(f);
  shown();
  return f;
}

const shown = () => { document.getElementById("none").hidden = box.children.length > 0; };

document.getElementById("add").onclick = () => add({}).elements.site.focus();

(async () => {
  const { rules = [], newRuleSite = "" } = await api.storage.local.get({ rules: [], newRuleSite: "" });
  for (const r of rules) add(r);
  // "Naming Rules…" from a page's list: that site's rule, made if there's none.
  if (newRuleSite) {
    await api.storage.local.remove("newRuleSite");
    const there = [...box.querySelectorAll(".rule")].find((f) => f.elements.site.value === newRuleSite);
    (there || add({ site: newRuleSite })).elements.pattern.focus();
    if (!there) await save();
  }
  shown();
})();
