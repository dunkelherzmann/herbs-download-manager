// scripts/guard-bash.mjs, the PreToolUse hook that blocks commands CLAUDE.md forbids: one blocked and some allowed
// commands per rule, in plain and RTK-rewritten form, then the hook itself run as Claude Code runs it.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { check } from "../../scripts/guard-bash.mjs";

// A stand-in home: the shared checkout (a .git folder) and a worktree of it (a .git file).
const home = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "hdm-guard-")));
const shared = path.join(home, "Projects", "herbs-download-manager");
const wt = path.join(home, "Projects", "wt-hdm-topic");
fs.mkdirSync(path.join(shared, ".git"), { recursive: true });
fs.mkdirSync(path.join(shared, "core"));
fs.mkdirSync(wt, { recursive: true });
fs.writeFileSync(path.join(wt, ".git"), `gitdir: ${shared}/.git/worktrees/wt-hdm-topic\n`);
process.on("exit", () => fs.rmSync(home, { recursive: true, force: true }));

const blocked = (cmd, cwd = wt) => assert.ok(check(cmd, cwd, home), `should block: ${cmd}`);
const allowed = (cmd, cwd = wt) => assert.equal(check(cmd, cwd, home), null, `should allow: ${cmd}`);

test("cargo fmt", () => {
    blocked("cargo fmt");
    blocked("rtk cargo fmt --all");
    blocked("cd core && cargo +nightly fmt");
    allowed("cargo fmt --check");
    allowed("cargo fmt --version");
    allowed("cargo test --workspace");
    allowed("rtk cargo build --release");
    allowed("grep -r 'cargo fmt' CLAUDE.md");
});

test("check.sh through tail or head", () => {
    blocked("scripts/check.sh | tail -5");
    blocked("sh scripts/check.sh 2>&1 | tail -20");
    blocked("rtk sh scripts/check.sh 2>&1 | grep -v x | rtk head -3");
    blocked("./scripts/check.sh |& tail");
    allowed("scripts/check.sh");
    allowed("sh scripts/check.sh; echo exit=$?");
    allowed("scripts/check.sh > /tmp/c.log 2>&1; tail -5 .build/check/linux.log");
    allowed("git log | tail -3");
});

test("pkill -f inside ssh", () => {
    blocked("ssh linux-box 'pkill -f hd-mock; cargo test'");
    blocked('ssh -p 22 linux-box "cd /tmp && pkill -9 -f hd-mock"');
    blocked("rtk ssh linux-box 'pkill -fx hd-mock'");
    allowed("ssh linux-box 'pkill -f [h]d-mock'");
    allowed("ssh linux-box 'kill $(cat /tmp/mock.pid)'");
    allowed("ssh linux-box 'pkill -x hd-mock'");
    allowed("pkill -f hd-mock");
});

test("pkill -f inside mc-exec", () => {
    blocked("mc-exec linux 'pkill -f hd-mock; cargo test'");
    blocked('mc-exec linux --cwd /tmp --timeout 60 "pkill -9 -f hd-mock"');
    blocked("rtk mc-exec linux 'pkill -fx hd-mock'");
    allowed("mc-exec linux 'pkill -f [h]d-mock'");
    allowed("mc-exec linux --put step.sh /tmp/step.sh");
});

test("commit, PR merge and branch switch in the shared checkout", () => {
    blocked("git commit -m x", shared);
    blocked("rtk git commit -am x", path.join(shared, "core"));
    blocked("cd ~/Projects/herbs-download-manager && git commit -m x", wt);
    blocked(`git -C ${shared} commit -m x`, wt);
    blocked("gh pr merge 12 --squash", shared);
    blocked("rtk gh pr merge 12 --squash --delete-branch", shared);
    blocked("git checkout some-branch", shared);
    blocked("git checkout -b new origin/main", shared);
    blocked("rtk git switch other", shared);
    blocked("gh pr checkout 12", shared);
    allowed("git commit -m x", wt);
    allowed("rtk gh pr merge 12 --squash", wt);
    allowed("git switch -c next", wt);
    allowed("cd ~/Projects/wt-hdm-topic && git commit -m x", shared);
    allowed("git status", shared);
    allowed("rtk git pull --ff-only", shared);
    allowed("git checkout main", shared);
    allowed("git checkout -- README.md", shared);
    allowed(`git -C ${shared} worktree add ~/Projects/wt-hdm-x -b x origin/main`, shared);
    allowed("ssh linux-box 'cd ~/Projects/herbs-download-manager && git checkout main'", shared);
});

test('an unquoted "Herb\'s" in ssh', () => {
    blocked(`ssh windows-box 'dir "%LOCALAPPDATA%\\Herb's Download Manager"'`);
    blocked(`ssh linux-box "ls ~/.local/share/Herb's\\ DM"`);
    blocked(`rtk ssh linux-box "rm -rf /tmp/Herb's"`);
    allowed(`ssh linux-box "ls \\"$HOME/Herb's Download Manager\\""`);
    allowed(`ssh windows-box "powershell -ExecutionPolicy Bypass -File C:\\t\\step.ps1"`);
    allowed(`ls "$HOME/Library/Application Support/Herb's Download Manager"`);
    allowed(`scp step.ps1 windows-box:step.ps1`);
});

test('an unquoted "Herb\'s" in mc-exec', () => {
    blocked(`mc-exec windows 'dir "%LOCALAPPDATA%\\Herb's Download Manager"'`);
    blocked(`mc-exec linux --timeout 60 "rm -rf /tmp/Herb's"`);
    allowed(`mc-exec linux "ls \\"$HOME/Herb's Download Manager\\""`);
    allowed(`mc-exec windows --put step.ps1 "C:\\t\\step.ps1"`);
});

test("the hook: exit 2 with a reason, 0 otherwise", () => {
    const script = fileURLToPath(new URL("../../scripts/guard-bash.mjs", import.meta.url));
    const run = (command, cwd) => spawnSync(process.execPath, [script], {
        input: JSON.stringify({ tool_name: "Bash", tool_input: { command }, cwd }),
        env: { ...process.env, HOME: home, USERPROFILE: home },
        encoding: "utf8",
    });
    const no = run("cargo fmt", wt);
    assert.equal(no.status, 2);
    assert.match(no.stderr, /^Blocked by scripts\/guard-bash\.mjs: cargo fmt is blocked/);
    assert.equal(no.stderr.trim().split("\n").length, 1);
    assert.equal(run("git commit -m x", shared).status, 2);
    assert.equal(run("git commit -m x", wt).status, 0);
    assert.equal(run("echo 'unbalanced", wt).status, 0);
    const bad = spawnSync(process.execPath, [script], { input: "not json", encoding: "utf8" });
    assert.equal(bad.status, 0);
});
