//! Finding addresses a page hides in its text (SPECIFICATION.md §15): written out in scripts and JSON with escaped
//! characters (`https:\/\/`, `/`, `\x2F`), as HTML entities, percent-encoded (`https%3A%2F%2F`), or in base64.
//! Players build their streams' addresses from such text, so the scan reads the page as they would.

use base64::Engine;

/// The most base64 runs decoded on one page, and the longest run tried.
const BASE64_MAX: usize = 400;
const BASE64_LONGEST: usize = 16 * 1024;

/// The page's text followed by every decoded form of it that differs, so addresses in any of them are found.
pub fn expand(html: &str) -> String {
    let mut out = String::with_capacity(html.len() * 2);
    out.push_str(html);
    let js = unescape_js(html);
    let entities = decode_entities(&js);
    for form in [&js, &entities] {
        if form.as_str() != html {
            out.push('\n');
            out.push_str(form);
        }
    }
    for found in percent_encoded(&entities).into_iter().chain(base64_addresses(&entities)).chain(unpacked(&js)) {
        out.push('\n');
        out.push_str(&found);
    }
    out
}

/// Scripts packed with the common `eval(function(p,a,c,k,e,d){…}('…',a,c,'w0|w1|…'.split('|')…))` packer, unpacked:
/// many video players are set up in such a script. Only the packer's word table is read; nothing is run.
pub fn unpacked(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = text[from..].find("eval(function(p,a,c,k,e,").map(|p| p + from) {
        from = at + 10;
        if out.len() >= 20 {
            break;
        }
        // The arguments follow the function's body: '}(' then the packed text in quotes.
        let Some(args) = text[at..].find("}('").or_else(|| text[at..].find("}(\"")).map(|p| at + p + 2) else { continue };
        let quote = text.as_bytes()[args];
        let body_start = args + 1;
        // The packed text ends at the first unescaped closing quote followed by ",".
        let bytes = text.as_bytes();
        let mut i = body_start;
        while i < bytes.len() && !(bytes[i] == quote && bytes[i - 1] != b'\\' && bytes.get(i + 1) == Some(&b',')) {
            i += 1;
        }
        if i >= bytes.len() {
            continue;
        }
        let packed = text[body_start..i].replace(&format!("\\{}", quote as char), &(quote as char).to_string());
        let rest = &text[i + 2..];
        let mut parts = rest.splitn(3, ',');
        let (Some(a), Some(c)) = (parts.next().and_then(|a| a.trim().parse::<u32>().ok()), parts.next().and_then(|c| c.trim().parse::<usize>().ok())) else { continue };
        let Some(tail) = parts.next().map(str::trim_start) else { continue };
        let Some(q) = tail.chars().next().filter(|q| *q == '\'' || *q == '"') else { continue };
        let Some(words) = tail[1..].split(q).next() else { continue };
        let words: Vec<&str> = words.split('|').collect();
        if !(2..=62).contains(&a) || c > 100_000 {
            continue;
        }
        // Replace each word token (its index written in base a) with its word.
        let mut result = String::with_capacity(packed.len() * 2);
        let pb = packed.as_bytes();
        let mut j = 0;
        while j < pb.len() {
            if pb[j].is_ascii_alphanumeric() || pb[j] == b'_' {
                let start = j;
                while j < pb.len() && (pb[j].is_ascii_alphanumeric() || pb[j] == b'_') {
                    j += 1;
                }
                let token = &packed[start..j];
                match from_base(token, a).and_then(|n| words.get(n)).filter(|w| !w.is_empty()) {
                    Some(word) => result.push_str(word),
                    None => result.push_str(token),
                }
            } else {
                let len = packed[j..].chars().next().map(char::len_utf8).unwrap_or(1);
                result.push_str(&packed[j..j + len]);
                j += len;
            }
        }
        out.push(unescape_js(&result));
    }
    out
}

/// A packer token's number: digits 0-9, then a-z, then A-Z, as the packer writes them in base `a`.
fn from_base(token: &str, a: u32) -> Option<usize> {
    let mut n: usize = 0;
    for c in token.chars() {
        let d = match c {
            '0'..='9' => c as u32 - '0' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 10,
            'A'..='Z' => c as u32 - 'A' as u32 + 36,
            _ => return None,
        };
        if d >= a {
            return None;
        }
        n = n.checked_mul(a as usize)?.checked_add(d as usize)?;
    }
    Some(n)
}

/// JavaScript and JSON escapes: `\/`, `\uXXXX` (with surrogate pairs) and `\xXX`; other backslashes stay.
pub fn unescape_js(s: &str) -> String {
    if !s.contains('\\') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let hex = |from: usize, n: usize| -> Option<u32> { s.get(from..from + n).and_then(|h| u32::from_str_radix(h, 16).ok()) };
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            match b[i + 1] {
                b'/' => {
                    out.push('/');
                    i += 2;
                    continue;
                }
                b'u' => {
                    if let Some(code) = hex(i + 2, 4) {
                        // A surrogate pair: 😀.
                        if (0xD800..0xDC00).contains(&code) && s.get(i + 6..i + 8) == Some("\\u") {
                            if let Some(low) = hex(i + 8, 4).filter(|l| (0xDC00..0xE000).contains(l)) {
                                if let Some(c) = char::from_u32(0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00)) {
                                    out.push(c);
                                    i += 12;
                                    continue;
                                }
                            }
                        }
                        if let Some(c) = char::from_u32(code) {
                            out.push(c);
                            i += 6;
                            continue;
                        }
                    }
                }
                b'x' => {
                    if let Some(c) = hex(i + 2, 2).and_then(char::from_u32) {
                        out.push(c);
                        i += 4;
                        continue;
                    }
                }
                _ => {}
            }
        }
        // Copy one whole character.
        let len = s[i..].chars().next().map(char::len_utf8).unwrap_or(1);
        out.push_str(&s[i..i + len]);
        i += len;
    }
    out
}

/// HTML entities: numeric ones (`&#47;`, `&#x2F;`) and the few named ones that turn up in addresses.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let end = tail[1..].find(';').map(|e| e + 1).filter(|e| *e <= 10);
        let decoded = end.and_then(|e| {
            let name = &tail[1..e];
            let c = match name {
                "amp" => '&',
                "quot" => '"',
                "apos" => '\'',
                "lt" => '<',
                "gt" => '>',
                "sol" => '/',
                "colon" => ':',
                "equals" => '=',
                "quest" => '?',
                _ if name.starts_with("#x") || name.starts_with("#X") => u32::from_str_radix(&name[2..], 16).ok().and_then(char::from_u32)?,
                _ if name.starts_with('#') => name[1..].parse::<u32>().ok().and_then(char::from_u32)?,
                _ => return None,
            };
            Some((c, e + 1))
        });
        match decoded {
            Some((c, used)) => {
                out.push(c);
                rest = &tail[used..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Addresses written percent-encoded (`https%3A%2F%2Fcdn…`, or twice: `https%253A%252F%252F…`), decoded.
fn percent_encoded(s: &str) -> Vec<String> {
    let lower = s.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = lower[from..].find("http").map(|p| p + from) {
        let tail = &lower[at..];
        let starts = ["http%3a%2f%2f", "https%3a%2f%2f", "http%253a%252f%252f", "https%253a%252f%252f"].iter().any(|p| tail.starts_with(p));
        if !starts {
            from = at + 4;
            continue;
        }
        let len = s[at..].find(|c: char| !(c.is_ascii_alphanumeric() || "%._~+-".contains(c))).unwrap_or(s.len() - at);
        let mut text = s[at..at + len].to_string();
        for _ in 0..2 {
            if text.contains('%') {
                text = percent_decode(&text);
            }
        }
        out.push(text);
        from = at + len;
    }
    out
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Base64 runs (at least 24 characters, standard or URL-safe) that decode to text holding an address.
fn base64_addresses(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut tried = 0;
    let is_b64 = |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'/' | b'-' | b'_' | b'=');
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && tried < BASE64_MAX {
        if !is_b64(b[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < b.len() && is_b64(b[i]) {
            i += 1;
        }
        let run = s[start..i].trim_end_matches('=');
        // Plain words and paths aren't base64: a run needs mixed case or digits, and no slashes runs of words.
        if run.len() < 24 || run.len() > BASE64_LONGEST || !run.bytes().any(|c| c.is_ascii_uppercase()) || !run.bytes().any(|c| c.is_ascii_lowercase()) {
            continue;
        }
        tried += 1;
        let engines = [&base64::engine::general_purpose::STANDARD_NO_PAD, &base64::engine::general_purpose::URL_SAFE_NO_PAD];
        for engine in engines {
            if let Ok(bytes) = engine.decode(run) {
                if let Ok(text) = String::from_utf8(bytes) {
                    if text.contains("://") && !text.contains('\0') {
                        out.push(text);
                    }
                }
                break;
            }
        }
    }
    out
}

/// Addresses that JSON or a player's setup names as media by its key (`"file":`, `"contentUrl":`, `"hls":`,
/// `"video_url":`, `sources: [{src: …}]`), whatever their address looks like. Read from already unescaped text.
pub fn media_values(text: &str) -> Vec<String> {
    const WORDS: &[&str] = &["video", "audio", "stream", "hls", "dash", "manifest", "playlist", "media", "mp4", "m3u8", "mpd", "contenturl", "file", "src", "source", "playable"];
    let mut out = Vec::new();
    let b = text.as_bytes();
    let mut from = 0;
    while let Some(at) = text[from..].find("http").map(|p| p + from) {
        from = at + 4;
        // The value's opening quote, then ":" or "=", then the key just before it.
        let Some(&quote) = at.checked_sub(1).and_then(|q| b.get(q)) else { continue };
        if !matches!(quote, b'"' | b'\'') {
            continue;
        }
        let before = text[..at - 1].trim_end();
        let Some(before) = before.strip_suffix(':').or_else(|| before.strip_suffix('=')) else { continue };
        let before = before.trim_end().trim_end_matches(['"', '\'']);
        let key: String = before.chars().rev().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect::<String>().chars().rev().collect();
        let key = key.to_ascii_lowercase();
        if key.is_empty() || !WORDS.iter().any(|w| key.contains(w)) {
            continue;
        }
        let value = &text[at..];
        let len = value.find(|c: char| c == quote as char || c.is_whitespace() || c == '<').unwrap_or(value.len());
        if value.starts_with("https://") || value.starts_with("http://") {
            out.push(value[..len].to_string());
        }
    }
    out
}

/// The addresses of a page's embedded frames and players (`<iframe src>`, `<frame>`, `<embed>`, `<object data>`,
/// and JSON-LD's `embedUrl`), in page order.
pub fn frame_sources(html: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    for (tag, attr) in [("<iframe", "src"), ("<iframe", "data-src"), ("<frame", "src"), ("<embed", "src"), ("<object", "data")] {
        let mut from = 0;
        while let Some(at) = lower[from..].find(tag).map(|p| p + from) {
            let end = lower[at..].find('>').map(|e| at + e).unwrap_or(lower.len());
            from = end.max(at + tag.len());
            let t = &lower[at..end];
            // "<frameset" isn't a frame.
            if t.as_bytes().get(tag.len()).is_some_and(|c| c.is_ascii_alphanumeric()) {
                continue;
            }
            if let Some(v) = attribute(&html[at..end], t, attr) {
                out.push((at, v));
            }
        }
    }
    out.sort_by_key(|(at, _)| *at);
    let mut seen = std::collections::HashSet::new();
    let mut list: Vec<String> = out.into_iter().map(|(_, v)| v).filter(|v| seen.insert(v.clone())).collect();
    let unescaped = unescape_js(html);
    for key in ["\"embedUrl\"", "\"embed_url\""] {
        let mut from = 0;
        while let Some(at) = unescaped[from..].find(key).map(|p| p + from) {
            from = at + key.len();
            let rest = unescaped[from..].trim_start().strip_prefix(':').map(str::trim_start).and_then(|r| r.strip_prefix('"'));
            if let Some(v) = rest.and_then(|r| r.split('"').next()) {
                if seen.insert(v.to_string()) {
                    list.push(v.to_string());
                }
            }
        }
    }
    list
}

/// One attribute's value in a tag (`lower` is the tag in lowercase).
fn attribute(tag: &str, lower: &str, name: &str) -> Option<String> {
    let mut from = 0;
    while let Some(p) = lower[from..].find(name).map(|p| p + from) {
        from = p + name.len();
        if p == 0 || !lower.as_bytes()[p - 1].is_ascii_whitespace() {
            continue;
        }
        let after = lower[from..].trim_start();
        let Some(after) = after.strip_prefix('=') else { continue };
        let at = tag.len() - after.len();
        let v = tag[at..].trim_start();
        let value = match v.chars().next() {
            Some(q @ ('"' | '\'')) => v[1..].split(q).next().unwrap_or(""),
            _ => v.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or(""),
        };
        let value = decode_entities(value.trim());
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_addresses_are_decoded() {
        let page = r#"<script>var a = "https://cdn.example.com/v/one.mp4";
            var b = 'https:\x2F\x2Fcdn.example.com\x2Fv\x2Ftwo.mp4'; var c = "https:\/\/cdn.example.com\/three.mp4";</script>
            <a data-x="https&#58;&#47;&#47;cdn.example.com&#x2F;four.mp4">x</a>
            <a href="/go?to=https%3A%2F%2Fcdn.example.com%2Ffive.mp4&amp;y=1">y</a> <a href="/r?u=https%253A%252F%252Fcdn.example.com%252Fsix.mp4">z</a>
            <div data-v="aHR0cHM6Ly9jZG4uZXhhbXBsZS5jb20vc2V2ZW4ubXA0"></div>"#;
        let text = expand(page);
        for n in ["one", "two", "three", "four", "five", "six", "seven"] {
            assert!(text.contains(&format!("https://cdn.example.com/{n}.mp4")) || text.contains(&format!("https://cdn.example.com/v/{n}.mp4")), "{n}: {text}");
        }
        assert_eq!(unescape_js(r"😀 \q"), "😀 \\q");
        assert_eq!(decode_entities("a&amp;b &unknown; &#x41;"), "a&b &unknown; A");
    }

    #[test]
    fn packed_player_scripts_are_unpacked() {
        // jwplayer("v").setup({file:"https://cdn.example.com/hls/abc/master.m3u8"}), packed with words 0..8.
        let packed = r#"<script>eval(function(p,a,c,k,e,d){e=function(c){return c};if(!''.replace(/^/,String)){while(c--){d[c]=k[c]||c}k=[function(e){return d[e]}];e=function(){return'\\w+'};c=1};while(c--){if(k[c]){p=p.replace(new RegExp('\\b'+e(c)+'\\b','g'),k[c])}}return p}('0("1").2({3:"4://5.6.7/8/a/b.c"})',13,13,'jwplayer|v|setup|file|https|cdn|example|com|hls||abc|master|m3u8'.split('|'),0,{}))</script>"#;
        let text = expand(packed);
        assert!(text.contains(r#"jwplayer("v").setup({file:"https://cdn.example.com/hls/abc/master.m3u8"})"#), "{text}");
        assert_eq!(from_base("z", 36), Some(35));
        assert_eq!(from_base("10", 62), Some(62));
    }

    #[test]
    fn media_keys_name_streams_without_extensions() {
        let text = r#"{"video_url":"https://cdn.example.com/get/abc?sig=1","author_url":"https://example.com/u/1"}
            jwplayer().setup({file: "https://cdn.example.com/hls/xyz", image: "https://cdn.example.com/p.jpg"});
            sources: [{src:'https://cdn.example.com/play/9', type:'video/mp4'}]"#;
        let found = media_values(text);
        assert_eq!(found, ["https://cdn.example.com/get/abc?sig=1", "https://cdn.example.com/hls/xyz", "https://cdn.example.com/play/9"]);
    }

    #[test]
    fn frames_and_embeds_are_listed() {
        let html = r#"<frameset><frame src="/top.html"></frameset><iframe width=1 SRC="https://player.example.net/e/42?x=1&amp;y=2"></iframe>
            <iframe data-src='//player.example.net/e/43'></iframe><embed src="/flash/old.swf">
            <script type="application/ld+json">{"embedUrl":"https:\/\/player.example.net\/e\/44"}</script>"#;
        assert_eq!(
            frame_sources(html),
            ["/top.html", "https://player.example.net/e/42?x=1&y=2", "//player.example.net/e/43", "/flash/old.swf", "https://player.example.net/e/44"]
        );
    }
}
