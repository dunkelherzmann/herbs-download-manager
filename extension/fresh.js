// Is the background script the one this extension's files hold? Edge can go on running an extension's old background
// script from its cache after the extension's files were replaced by a newer version, and the old one never answers
// what it doesn't know (the page's button stayed busy and nothing reached the app, bugs.md 2026-11-16). Loaded by the
// extension's own pages (the list, the options page, restart.html): when "hello" gets no answer within a few seconds,
// or names another version than manifest.json, the extension is restarted, which loads its files anew.

(() => {
  const api = globalThis.browser ?? chrome;
  const WAIT = 4000;
  const asked = Promise.race([
    api.runtime.sendMessage({ type: "hello" }).catch(() => null),
    new Promise((done) => setTimeout(() => done(null), WAIT)),
  ]);
  globalThis.herbsDmFresh = asked.then((answer) => {
    const fresh = !!answer && answer.script === api.runtime.getManifest().version;
    if (!fresh) api.runtime.reload();
    return fresh;
  });
})();
