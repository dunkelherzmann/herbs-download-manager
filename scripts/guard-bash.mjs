#!/usr/bin/env node
// Claude Code PreToolUse hook (.claude/settings.json, matcher Bash): turns the hard-won rules in CLAUDE.md into hard
// stops. It reads the hook's JSON on stdin and exits 2 with a one-line reason when the command would
//   - run `cargo fmt` (no rustfmt.toml: it rewrites the whole workspace);
//   - pipe scripts/check.sh into tail or head (the pipe's status is tail's: #89 merged with machines failing);
//   - run `pkill -f` inside `ssh host '…'` (the remote shell's command line matches the pattern: it kills itself);
//   - commit, merge a PR or switch branches in the shared checkout ~/Projects/herbs-download-manager instead of a
//     wt-hdm-* worktree (another session switched the branch under a running task: #64);
//   - send an ssh command with an unquoted "Herb's" (the apostrophe ends the quoting and vanishes).
// RTK's global hook rewrites commands (`git status` → `rtk git status`), so a leading `rtk` is looked through.
// Anything it can't parse is let through: the hook must never stop ordinary work.
// Tests: node --test tests/guard/*.test.mjs
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const SEPARATORS = new Set([";", "&&", "||", "&", "\n"]);

// Splits shell text into words ({value, raw}) and operators ({op}). Quotes and backslashes are removed from value
// as the shell would; $(…) and `…` stay inside their word. Throws on an unterminated quote.
export function lex(text) {
    const out = [];
    let i = 0;
    let word = null;
    const start = () => { if (!word) word = { value: "", raw: "", quoted: false }; };
    const end = () => { if (word) { out.push(word); word = null; } };
    while (i < text.length) {
        const c = text[i];
        if (c === " " || c === "\t") { end(); i++; continue; }
        if (c === "\\" && text[i + 1] === "\n") { i += 2; continue; }
        if (c === "\n" || c === ";" || c === "(" || c === ")") { end(); out.push({ op: c }); i++; continue; }
        if (c === "&" || c === "|") {
            // >&2, 2>&1, &>file belong to a redirection, not a separator.
            if (c === "&" && (word && /[<>]$/.test(word.raw) || text[i + 1] === ">")) { start(); word.raw += c; word.value += c; i++; continue; }
            end();
            const two = text.slice(i, i + 2);
            if (two === "&&" || two === "||" || two === "|&") { out.push({ op: two }); i += 2; } else { out.push({ op: c }); i++; }
            continue;
        }
        if (c === "#" && !word) { while (i < text.length && text[i] !== "\n") i++; continue; }
        start();
        if (c === "'") {
            const j = text.indexOf("'", i + 1);
            if (j < 0) throw new Error("unterminated '");
            word.value += text.slice(i + 1, j); word.raw += text.slice(i, j + 1); word.quoted = true; i = j + 1;
        } else if (c === '"') {
            let j = i + 1;
            let v = "";
            while (j < text.length && text[j] !== '"') {
                if (text[j] === "\\" && '"\\$`\n'.includes(text[j + 1])) { v += text[j + 1]; j += 2; } else { v += text[j]; j++; }
            }
            if (j >= text.length) throw new Error('unterminated "');
            word.value += v; word.raw += text.slice(i, j + 1); word.quoted = true; i = j + 1;
        } else if (c === "\\") {
            word.value += text[i + 1] ?? ""; word.raw += text.slice(i, i + 2); i += 2;
        } else if (c === "$" && text[i + 1] === "(") {
            let depth = 0, j = i + 1;
            for (; j < text.length; j++) { if (text[j] === "(") depth++; else if (text[j] === ")" && --depth === 0) break; }
            word.value += text.slice(i, j + 1); word.raw += text.slice(i, j + 1); i = j + 1;
        } else {
            word.value += c; word.raw += c; i++;
        }
    }
    end();
    return out;
}

// The words of each simple command, with the operator that ended it, in order.
function commands(tokens) {
    const list = [];
    let words = [];
    for (const t of tokens) {
        if (t.op) { list.push({ words, end: t.op }); words = []; } else words.push(t);
    }
    list.push({ words, end: null });
    return list.filter((c) => c.words.length || c.end === "|" || c.end === "|&");
}

// Drops what runs another command unchanged: env assignments, rtk (and rtk proxy), env, command, time, nohup, sudo.
function strip(words) {
    let w = words;
    for (;;) {
        if (!w.length) return w;
        const v = w[0].value;
        if (/^[A-Za-z_][A-Za-z0-9_]*=/.test(v) && !w[0].raw.startsWith("=")) { w = w.slice(1); continue; }
        if (v === "rtk" && w[1]?.value === "proxy") { w = w.slice(2); continue; }
        if (["rtk", "env", "command", "time", "nohup", "sudo", "exec"].includes(v)) { w = w.slice(1); continue; }
        return w;
    }
}

const expand = (word, home) => {
    // An unquoted Windows path (C:\Users\…) loses its backslashes to the shell's rules; it still names that folder.
    let v = /^[A-Za-z]:\\[^'"]*$/.test(word.raw) ? word.raw : word.value;
    if (!word.raw.startsWith("'") && !word.raw.startsWith('"')) v = v.replace(/^~(?=\/|$)/, home);
    return v.replace(/^\$\{?HOME\}?(?=\/|$)/, home);
};

// The folder of the checkout that holds dir, and whether it's a main checkout (.git is a folder there; a worktree
// has a .git file).
function checkoutOf(dir) {
    let d = dir;
    for (let n = 0; n < 64; n++) {
        try {
            const st = fs.statSync(path.join(d, ".git"));
            return { top: d, main: st.isDirectory() };
        } catch { /* keep looking */ }
        const up = path.dirname(d);
        if (up === d) return null;
        d = up;
    }
    return null;
}

const realish = (p) => { try { return fs.realpathSync(p); } catch { return path.resolve(p); } };
const same = (a, b) => (process.platform === "win32" ? a.toLowerCase() === b.toLowerCase() : a === b);

const SSH_ARG_OPTS = new Set("BbcDEeFIiJLlmOoPpQRSWw".split(""));

// ssh's host and the remote command (its words after the host, joined by spaces as ssh sends them).
function sshRemote(words) {
    let i = 1;
    while (i < words.length && words[i].value.startsWith("-")) {
        const opt = words[i].value;
        const last = opt[opt.length - 1];
        i += opt.length === 2 && SSH_ARG_OPTS.has(last) ? 2 : 1;
    }
    if (i >= words.length) return null;
    return words.slice(i + 1).map((w) => w.value).join(" ");
}

// True when the text, as a shell reads it, has an apostrophe of "Herb's" outside double quotes and unescaped.
function unquotedHerbs(text) {
    let dq = false;
    for (let i = 0; i < text.length; i++) {
        const c = text[i];
        if (c === "\\") { i++; continue; }
        if (c === '"') { dq = !dq; continue; }
        if (!dq && c === "'") {
            if (/Herb$/i.test(text.slice(0, i)) && /^s\b/.test(text.slice(i + 1))) return true;
            const j = text.indexOf("'", i + 1); // a single-quoted part: skip it
            if (j < 0) return false;
            i = j;
        }
    }
    return false;
}

function pkillF(words) {
    if (words[0]?.value !== "pkill") return false;
    for (let i = 1; i < words.length; i++) {
        const v = words[i].value;
        if (v === "--full" || (/^-[A-Za-z]+$/.test(v) && v.includes("f"))) {
            // A pattern that can't match its own command line ('[h]d-mock') is the safe way.
            const pattern = words.slice(i + 1).find((w) => !w.value.startsWith("-"));
            return !(pattern && /^\[.\]/.test(pattern.value));
        }
    }
    return false;
}

const SHARED = "the shared checkout ~/Projects/herbs-download-manager: work in your own worktree (git worktree add ~/Projects/wt-hdm-<topic> -b <branch> origin/main); another session switched the branch here under a running task (#64)";

// Returns the reason to block the command, or null to let it run.
export function check(command, cwd, home = os.homedir(), depth = 0) {
    const shared = realish(path.join(home, "Projects", "herbs-download-manager"));
    let tokens;
    try { tokens = lex(command); } catch {
        // The outer command isn't even valid: only an ssh line broken by "Herb's" is worth naming.
        return /\bssh\b/.test(command) && /Herb's/i.test(command) ? herbsReason : null;
    }
    const cmds = commands(tokens);
    let dir = cwd;
    for (let n = 0; n < cmds.length; n++) {
        const words = strip(cmds[n].words);
        if (!words.length) continue;
        const name = path.basename(words[0].value);
        const v = words.map((w) => w.value);

        if (name === "cd" && cmds[n].end !== "|") {
            const to = words[1] ? expand(words[1], home) : home;
            if (to !== "-") dir = path.resolve(dir, to);
            continue;
        }
        if (name === "cargo") {
            const sub = v[1]?.startsWith("+") ? v[2] : v[1];
            if (sub === "fmt" && !v.some((x) => ["--check", "--version", "-V", "--help", "-h"].includes(x))) {
                return "cargo fmt is blocked: there is no rustfmt.toml and the code isn't rustfmt-formatted, so it rewrites the whole workspace (CLAUDE.md)";
            }
        }
        // scripts/check.sh (or sh scripts/check.sh) piped into tail or head, later in the same pipeline.
        const isCheck = name === "check.sh" || (["sh", "bash", "zsh"].includes(name) && v.slice(1).some((x) => path.basename(x) === "check.sh"));
        if (isCheck) {
            for (let m = n; m < cmds.length && (cmds[m].end === "|" || cmds[m].end === "|&"); m++) {
                const next = strip(cmds[m + 1]?.words ?? []);
                if (next.length && ["tail", "head"].includes(path.basename(next[0].value))) {
                    return "scripts/check.sh piped into tail/head is blocked: the pipe returns tail's status, and #89 merged with Linux and Windows failing. Run it plain, judge its exit status and one ok line per machine; read .build/check/<machine>.log for detail";
                }
            }
        }
        if (["sh", "bash", "zsh"].includes(name) && v[1] === "-c" && v[2] != null && depth < 3) {
            const r = check(v[2], dir, home, depth + 1);
            if (r) return r;
        }
        if (name === "ssh") {
            const remote = sshRemote(words);
            if (remote) {
                if (unquotedHerbs(remote)) return herbsReason;
                let inner = null;
                try { inner = commands(lex(remote)); } catch { /* not sh syntax (cmd): nothing more to check */ }
                if (inner && inner.some((c) => pkillF(strip(c.words)))) {
                    return "pkill -f inside ssh host '…' is blocked: the remote shell's own command line contains the pattern, so it kills itself and the later steps silently don't run. Save the PID when starting (echo $! > file) and kill that, or write the pattern as '[h]d-mock'";
                }
            }
            continue;
        }
        // Commits, PR merges and branch switches in the shared checkout.
        let gdir = dir;
        let sub = null, rest = [];
        if (name === "git") {
            let i = 1;
            while (i < words.length && v[i].startsWith("-")) {
                if (v[i] === "-C") { gdir = path.resolve(gdir, expand(words[i + 1], home)); i += 2; } else if (v[i] === "-c") i += 2; else i++;
            }
            sub = v[i]; rest = v.slice(i + 1);
        }
        let risky = null;
        if (sub === "commit") risky = "git commit";
        if (sub === "switch" || (sub === "checkout" && !rest.includes("--"))) {
            // Back to main is how the shared checkout should be; anything else moves it under other sessions.
            const args = rest.filter((x) => !x.startsWith("-"));
            const toMain = args.length === 1 && args[0] === "main" && !rest.some((x) => /^-[bBc]$|^--orphan$|^--detach$/.test(x));
            if (!toMain) risky = `git ${sub}`;
        }
        if (name === "gh" && v[1] === "pr" && (v[2] === "merge" || v[2] === "checkout")) {
            risky = `gh pr ${v[2]}`;
            for (let i = 3; i < v.length; i++) if (v[i] === "-R" || v[i] === "--repo") i++;
        }
        if (risky) {
            const co = checkoutOf(gdir);
            if (co && co.main && same(realish(co.top), shared)) return `${risky} in ${SHARED}`;
        }
    }
    return null;
}

const herbsReason = "an unquoted \"Herb's\" in an ssh command is blocked: the apostrophe ends the shell's quoting and vanishes (a command made a new \"Herbs Download Manager\" folder). Put the step in a .ps1/.sh (Join-Path $env:LOCALAPPDATA \"Herb's Download Manager\"), copy it over with scp and run that (docs/agents/linux-windows.md)";

if (process.argv[1] && realish(process.argv[1]) === realish(fileURLToPath(import.meta.url))) {
    let input = "";
    process.stdin.setEncoding("utf8");
    process.stdin.on("data", (d) => { input += d; });
    process.stdin.on("end", () => {
        let reason = null;
        try {
            const hook = JSON.parse(input);
            const command = hook?.tool_input?.command;
            if (typeof command === "string") reason = check(command, hook.cwd || process.cwd());
        } catch { reason = null; }
        if (reason) { process.stderr.write(`Blocked by scripts/guard-bash.mjs: ${reason}\n`); process.exit(2); }
        process.exit(0);
    });
}
