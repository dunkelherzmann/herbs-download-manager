// The naming rules' own functions (extension/naming.js, SPECIFICATION.md §13).

const test = require("node:test");
const assert = require("node:assert/strict");
const { ruleFor, siteName, cleanName, nameByRule, cleanFolder } = require("../../extension/naming.js");

test("a rule covers its site and subdomains; the most particular wins", () => {
  const rules = [{ site: "example.com", n: 1 }, { site: "video.example.com", n: 2 }, { site: "www.other.org", n: 3 }];
  assert.equal(ruleFor(rules, "https://www.example.com/a").n, 1);
  assert.equal(ruleFor(rules, "https://video.example.com/a").n, 2);
  assert.equal(ruleFor(rules, "https://other.org/").n, 3, "www. is left out of the rule's site");
  assert.equal(ruleFor(rules, "https://notexample.com/"), null);
  assert.equal(ruleFor(rules, "not a link"), null);
});

test("a site's name, a clean file name and a sub-folder", () => {
  assert.equal(siteName("https://www.youtube.com/watch"), "youtube");
  assert.equal(siteName("https://www.bbc.co.uk/x"), "bbc");
  assert.equal(cleanName("  ..a/b:c*d  \n e "), "a b c d e");
  assert.equal(cleanName("///"), "no-name");
  assert.equal(cleanFolder("../Music//Live/./x:y"), "Music/Live/x y");
});

test("a rule's name: pattern, pairs in order, length", () => {
  const page = { title: "Song (Official Video) | Band", url: "https://m.youtube.com/watch", selected: "Channel" };
  assert.equal(nameByRule({ pattern: "%site - %title", replace: [["(Official Video)", ""]] }, page), "youtube - Song Band");
  assert.equal(nameByRule({ pattern: "%selector: %title", replace: [["Song", "Tune"], ["Tune", "Air"]] }, page), "Channel Air (Official Video) Band");
  assert.equal(nameByRule({ max: 4 }, page), "Song");
  assert.equal(nameByRule({}, { title: "", url: "https://a.com" }), "no-name");
});
