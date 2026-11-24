//! Finding links in pasted or dropped text (SPECIFICATION.md §5.2–§5.4) and expanding batch patterns (§5.5).

mod documents;

/// The text of a dropped or imported file (§5.4, §5.7) for the hosts that read it themselves (Linux, Windows), as the
/// Mac reads it: plain text, HTML and rich text (UTF-8 or UTF-16, by its byte-order mark); `.docx`, `.odt`, `.doc`,
/// `.rtfd`, `.webarchive` and `.webloc` files, their hyperlinks' targets included. `None` for a file that isn't text
/// (a NUL in its first 8 KB: a zip, an image), a folder other than an `.rtfd`, or a file over 50 MB.
pub fn text_of_file(path: &std::path::Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if meta.is_dir() {
        // An .rtfd is a folder holding the document as TXT.rtf.
        return if ext == "rtfd" { text_of_file(&path.join("TXT.rtf")) } else { None };
    }
    if !meta.is_file() || meta.len() > 50 * 1024 * 1024 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    match ext.as_str() {
        "docx" => documents::docx(&bytes),
        "odt" => documents::odt(&bytes),
        "doc" => documents::doc(&bytes),
        "webloc" => documents::webloc(&bytes),
        "webarchive" => documents::webarchive(&bytes),
        _ => plain_text(&bytes),
    }
}

/// UTF-8 or UTF-16 text (by its byte-order mark); `None` when the first 8 KB hold a NUL byte (a binary file).
fn plain_text(bytes: &[u8]) -> Option<String> {
    let utf16 = |le: bool| {
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| if le { u16::from_le_bytes(c) } else { u16::from_be_bytes(c) })
            .collect();
        Some(String::from_utf16_lossy(&units))
    };
    match bytes {
        [0xFF, 0xFE, ..] => utf16(true),
        [0xFE, 0xFF, ..] => utf16(false),
        _ if bytes.iter().take(8192).any(|&b| b == 0) => None,
        _ => Some(String::from_utf8_lossy(bytes).into_owned()),
    }
}

/// Export Links (§5.8): each file's name with its link and its mirrors under it, drawn as a tree (as `tree` draws a
/// folder). Read back by Import Links, whose Add Links window gathers links to the same file into one download again.
pub fn tree(files: &[(String, Vec<String>)]) -> String {
    let mut out = String::from(".\n");
    for (i, (name, links)) in files.iter().enumerate() {
        let last = i + 1 == files.len();
        out.push_str(if last { "└── " } else { "├── " });
        out.push_str(name);
        out.push('\n');
        for (j, link) in links.iter().enumerate() {
            out.push_str(if last { "    " } else { "│   " });
            out.push_str(if j + 1 == links.len() { "└── " } else { "├── " });
            out.push_str(link);
            out.push('\n');
        }
    }
    out
}

/// Every http, https and ftp link in plain text or HTML, in order, without repeats. Trailing punctuation that
/// belongs to the sentence (".", ",", ")", quotes) is left out. As JDownloader reads a paste: `hxxp://` and `h**p://`
/// (written so a link isn't clickable) count as `http://`; links run together (`…a.ziphttps://…`) are split; a link
/// inside another's query (`?u=https%3A%2F%2F…`) is found too, after it; `www.` addresses count without their
/// `http://`, and so does a paste that is only one `domain.tld/path`.
pub fn find(text: &str) -> Vec<String> {
    let text = &unmask(text);
    let mut found: Vec<(usize, String)> = Vec::new();
    let lower = text.to_ascii_lowercase();
    let mut i = 0;
    while i < text.len() {
        let rest = &lower[i..];
        let Some(at) = ["https://", "http://", "ftp://"].iter().filter_map(|p| rest.find(p)).min() else { break };
        let start = i + at;
        let end = link_end(text, start);
        // Run together: a scheme right after a letter or digit starts the next link.
        let glued = ["https://", "http://", "ftp://"]
            .iter()
            .filter_map(|p| lower[start + 1..end].find(p).map(|n| start + 1 + n))
            .filter(|&n| text[..n].chars().last().is_some_and(|c| c.is_ascii_alphanumeric()))
            .min();
        let end = glued.unwrap_or(end);
        if let Some(link) = clean(&text[start..end]) {
            let inner = inner_links(&link);
            found.push((start, link));
            found.extend(inner.into_iter().map(|l| (start, l)));
        }
        i = end.max(start + 1);
    }
    // `www.` addresses without a scheme: a word that starts with "www.".
    let mut from = 0;
    while let Some(n) = lower[from..].find("www.") {
        let start = from + n;
        from = start + 4;
        if text[..start].chars().last().is_some_and(|c| c.is_alphanumeric() || "./-:@_%=".contains(c)) {
            continue;
        }
        let end = link_end(text, start);
        if let Some(link) = clean(&format!("http://{}", &text[start..end])).filter(|l| has_real_host(l)) {
            found.push((start, link));
        }
    }
    // The whole paste is one address without a scheme: "example.com/files/a.zip".
    let trimmed = text.trim();
    if found.is_empty() && !trimmed.contains(char::is_whitespace) && trimmed.contains('/') && !trimmed.starts_with('/') {
        if let Some(link) = clean(&format!("http://{trimmed}")).filter(|l| has_real_host(l)) {
            found.push((0, link));
        }
    }
    found.sort_by_key(|(at, _)| *at);
    let mut out: Vec<String> = Vec::new();
    for (_, link) in found {
        if !out.contains(&link) {
            out.push(link);
        }
    }
    out
}

/// `hxxp://`, `hxxps://`, `h**p://` and the like, written so a link isn't clickable, as `http(s)://`.
fn unmask(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let rest = &lower[i..];
        let masked = rest.len() >= 7
            && rest.as_bytes()[0] == b'h'
            && rest.as_bytes()[1..3].iter().all(|b| matches!(b, b'x' | b'*' | b't'))
            && &rest[1..3] != "tt"
            && rest.as_bytes()[3] == b'p'
            && (rest[4..].starts_with("://") || rest[4..].starts_with("s://"));
        if masked && !text[..i].chars().last().is_some_and(|c| c.is_ascii_alphanumeric()) {
            out.push_str("http");
            i += 4;
        } else {
            let c = text[i..].chars().next().unwrap_or(' ');
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

/// Where a link that starts at `start` ends: at a space or a character links don't hold.
fn link_end(text: &str, start: usize) -> usize {
    text[start..]
        .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`' | '[' | ']' | '{' | '}' | '|' | '\\'))
        .map_or(text.len(), |n| start + n)
}

/// A link without the sentence's punctuation after it, and with `&amp;` as `&`; `None` if it isn't one.
fn clean(raw: &str) -> Option<String> {
    let mut link = raw;
    while let Some(last) = link.chars().last() {
        let unbalanced_paren = last == ')' && link.matches('(').count() < link.matches(')').count();
        if matches!(last, '.' | ',' | ';' | ':' | '!' | '?') || unbalanced_paren {
            link = &link[..link.len() - last.len_utf8()];
        } else {
            break;
        }
    }
    let link = link.replace("&amp;", "&");
    reqwest::Url::parse(&link).is_ok_and(|u| u.host_str().is_some()).then_some(link)
}

/// Links carried in another link's query, such as a redirect's `?url=https%3A%2F%2Fcdn.example.com%2Ff.zip`.
fn inner_links(link: &str) -> Vec<String> {
    let Ok(url) = reqwest::Url::parse(link) else { return Vec::new() };
    url.query_pairs()
        .map(|(_, v)| v.into_owned())
        .filter(|v| v.starts_with("http://") || v.starts_with("https://"))
        .filter_map(|v| clean(&v))
        .collect()
}

/// A host written as a name with a dot and a top-level domain of letters ("example.com"), or an IP address: what a
/// link without its scheme must have to count.
fn has_real_host(link: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(link) else { return false };
    match url.domain() {
        Some(d) => {
            let labels: Vec<&str> = d.split('.').collect();
            labels.len() >= 2
                && labels.iter().all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
                && labels.last().is_some_and(|t| t.len() >= 2 && t.chars().all(|c| c.is_ascii_alphabetic()))
        }
        None => url.host_str().is_some(),
    }
}

/// Copied HTML with its relative links (`href="files/a.zip"`, `src=…`) made whole against the page it was copied from
/// (`base`, the address the browser puts with a copy), so that they are found as links too.
pub fn absolute_html(html: &str, base: &str) -> String {
    let Ok(base) = reqwest::Url::parse(base) else { return html.to_string() };
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut i = 0;
    while i < html.len() {
        let next = ["href=", "src="].iter().filter_map(|a| lower[i..].find(a).map(|n| (i + n, a.len()))).min();
        let Some((at, len)) = next else { break };
        let value_at = at + len;
        out.push_str(&html[i..value_at]);
        let quote = html[value_at..].chars().next().filter(|c| *c == '"' || *c == '\'');
        let body = value_at + quote.map_or(0, |_| 1);
        let end = match quote {
            Some(q) => html[body..].find(q).map_or(html.len(), |n| body + n),
            None => html[body..].find(|c: char| c.is_whitespace() || c == '>').map_or(html.len(), |n| body + n),
        };
        let value = &html[body..end];
        let whole = reqwest::Url::parse(value).is_ok();
        let skip = value.starts_with('#') || value.to_ascii_lowercase().starts_with("javascript:") || value.to_ascii_lowercase().starts_with("mailto:");
        out.push_str(&html[value_at..body]);
        match base.join(&value.replace("&amp;", "&")) {
            Ok(u) if !whole && !skip && !value.is_empty() => out.push_str(u.as_str()),
            _ => out.push_str(value),
        }
        i = end;
    }
    out.push_str(&html[i..]);
    out
}

/// The page a copy was made from, from the address a browser stores beside copied HTML (Chromium's
/// `chromium/x-source-url`, Firefox's `text/x-moz-url-priv` on Linux, which is UTF-16, or `SourceURL:` on Windows):
/// UTF-8 or UTF-16 text, its first line, if it is a web or ftp address.
pub fn copied_from(data: &[u8]) -> Option<String> {
    let text = if data.len() >= 2 && data.len() % 2 == 0 && (data.starts_with(&[0xff, 0xfe]) || data[1] == 0) {
        let units: Vec<u16> = data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(data).into_owned()
    };
    let line = text.trim_start_matches('\u{feff}').lines().next()?.trim_matches(|c: char| c.is_whitespace() || c == '\0');
    let url = reqwest::Url::parse(line).ok()?;
    matches!(url.scheme(), "http" | "https" | "ftp").then(|| line.to_string())
}

/// One `^(…)` placeholder's values.
fn expand_range(spec: &str) -> Result<Vec<String>, String> {
    let bad = || format!("“{spec}” isn't a range like ###1-100, 0-ffH or a-z");
    let mut values = Vec::new();
    for part in spec.split(',').map(str::trim) {
        let (body, hex) = match part.strip_suffix(['H', 'h']) {
            Some(b) if b.contains('-') => (b, true),
            _ => (part, false),
        };
        let pad = body.chars().take_while(|&c| c == '#').count();
        let body = &body[pad..];
        let (a, b) = body.split_once('-').ok_or_else(bad)?;
        let (a, b) = (a.trim(), b.trim());
        let numeric = |s: &str| if hex { s.chars().all(|c| c.is_ascii_hexdigit()) } else { s.chars().all(|c| c.is_ascii_digit()) };
        if !a.is_empty() && !b.is_empty() && numeric(a) && numeric(b) {
            let radix = if hex { 16 } else { 10 };
            let from = u64::from_str_radix(a, radix).map_err(|_| bad())?;
            let to = u64::from_str_radix(b, radix).map_err(|_| bad())?;
            let width = pad.max(if a.starts_with('0') && a.len() > 1 { a.len() } else { 0 });
            let upper = hex && a.chars().chain(b.chars()).any(|c| c.is_ascii_uppercase());
            let step: Box<dyn Iterator<Item = u64>> = if from <= to { Box::new(from..=to) } else { Box::new((to..=from).rev()) };
            for n in step {
                let text = match (hex, upper) {
                    (true, true) => format!("{n:0width$X}"),
                    (true, false) => format!("{n:0width$x}"),
                    _ => format!("{n:0width$}"),
                };
                values.push(text);
                if values.len() > 100_000 {
                    return Err("That makes more than 100,000 links".into());
                }
            }
        } else if !a.is_empty() && a.len() == b.len() && a.chars().chain(b.chars()).all(|c| c.is_ascii_alphabetic()) {
            // Letters count like digits in base 26: a-z, AA-BZ.
            let upper = a.chars().all(|c| c.is_ascii_uppercase());
            let base = if upper { b'A' } else { b'a' };
            let num = |s: &str| s.to_ascii_lowercase().bytes().fold(0u64, |n, c| n * 26 + u64::from(c - b'a'));
            let (from, to) = (num(a), num(b));
            if from > to {
                return Err(bad());
            }
            for mut n in from..=to {
                let mut chars = vec![b'a'; a.len()];
                for slot in chars.iter_mut().rev() {
                    *slot = base + (n % 26) as u8;
                    n /= 26;
                }
                values.push(String::from_utf8(chars).unwrap_or_default());
                if values.len() > 100_000 {
                    return Err("That makes more than 100,000 links".into());
                }
            }
        } else {
            return Err(bad());
        }
    }
    Ok(values)
}

/// Batch Download (§5.5): `https://example.com/img^(###1-100).jpg` gives img001.jpg … img100.jpg. Several
/// placeholders multiply.
pub fn expand_batch(pattern: &str) -> Result<Vec<String>, String> {
    let mut links = vec![String::new()];
    let mut rest = pattern.trim();
    let mut found = false;
    while let Some(at) = rest.find("^(") {
        let close = rest[at..].find(')').ok_or("A “^(” has no closing “)”")? + at;
        let values = expand_range(&rest[at + 2..close])?;
        let prefix = &rest[..at];
        links = links.iter().flat_map(|l| values.iter().map(move |v| format!("{l}{prefix}{v}"))).collect();
        if links.len() > 100_000 {
            return Err("That makes more than 100,000 links".into());
        }
        rest = &rest[close + 1..];
        found = true;
    }
    if !found {
        return Err("Put a range like ^(###1-100) where the number goes".into());
    }
    let links: Vec<String> = links.into_iter().map(|l| l + rest).collect();
    if links.first().is_some_and(|l| reqwest::Url::parse(l).is_err()) {
        return Err("That isn't a web link".into());
    }
    Ok(links)
}

/// A column of Add Links' list (§5.3) that its rows can be sorted by.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LinkColumn {
    Name,
    Size,
    Folder,
    Link,
}

/// What Add Links sorts one row by.
pub struct LinkRow<'a> {
    pub name: &'a str,
    pub size: Option<u64>,
    pub folder: &'a str,
    pub url: &'a str,
}

/// The order of Add Links' rows sorted by a column, as indexes into `rows`: text A to Z, ignoring case, with numbers
/// by value ("part 2" before "part 10"); sizes by bytes, smallest first, unknown sizes before any. `ascending` false
/// gives Z to A and biggest first. Rows that tie are ordered by name, then keep their order.
pub fn sort_order(rows: &[LinkRow], by: LinkColumn, ascending: bool) -> Vec<usize> {
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by(|&a, &b| {
        let (x, y) = (&rows[a], &rows[b]);
        let key = match by {
            LinkColumn::Name => natural_cmp(x.name, y.name),
            LinkColumn::Size => x.size.cmp(&y.size),
            LinkColumn::Folder => natural_cmp(x.folder, y.folder),
            LinkColumn::Link => natural_cmp(x.url, y.url),
        };
        let key = if by == LinkColumn::Name { key } else { key.then_with(|| natural_cmp(x.name, y.name)) };
        if ascending { key } else { key.reverse() }
    });
    order
}

/// Compares text the way a person sorts names: case aside, and runs of digits by their value.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let digits = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut run = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        run.push(c);
                        it.next();
                    }
                    run.trim_start_matches('0').to_string()
                };
                let (m, n) = (digits(&mut a), digits(&mut b));
                let order = m.len().cmp(&n.len()).then_with(|| m.cmp(&n));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                let order = x.to_lowercase().cmp(y.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                a.next();
                b.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exported_links_are_a_tree_import_reads_back() {
        let files = vec![
            ("Big Buck Bunny.mp4".to_string(), vec!["https://a.example/bbb.mp4".to_string(), "https://b.example/m/bbb.mp4".into()]),
            ("notes.zip".to_string(), vec!["ftp://c.example/notes.zip".to_string()]),
        ];
        let text = tree(&files);
        assert_eq!(text, "\
.
├── Big Buck Bunny.mp4
│   ├── https://a.example/bbb.mp4
│   └── https://b.example/m/bbb.mp4
└── notes.zip
    └── ftp://c.example/notes.zip
");
        assert_eq!(find(&text), ["https://a.example/bbb.mp4", "https://b.example/m/bbb.mp4", "ftp://c.example/notes.zip"]);
        assert_eq!(tree(&[]), ".\n");
    }

    #[test]
    fn sorts_add_links_rows_by_each_column() {
        let row = |name, size, url| LinkRow { name, size, folder: "~/Downloads", url };
        let rows = [
            row("preview.mp4", Some(189_000), "https://t.example/b/preview.mp4"),
            row("Mozaique_listing.jpg", Some(90_600), "https://t.example/a/mozaique_listing.jpg"),
            row("part 10.bin", None, "https://t.example/c/part10"),
            row("part 2.bin", Some(1_000_000), "https://t.example/c/part2"),
            row("preview.mp4", Some(190_000), "https://t.example/a/preview.mp4"),
        ];
        // Names A to Z, case aside and numbers by value; equal names keep their order.
        assert_eq!(sort_order(&rows, LinkColumn::Name, true), [1, 3, 2, 0, 4]);
        assert_eq!(sort_order(&rows, LinkColumn::Name, false), [0, 4, 2, 3, 1]);
        // Sizes by bytes, not by their text ("90.6 KB" after "189 KB" as text); unknown first.
        assert_eq!(sort_order(&rows, LinkColumn::Size, true), [2, 1, 0, 4, 3]);
        assert_eq!(sort_order(&rows, LinkColumn::Size, false), [3, 4, 0, 1, 2]);
        assert_eq!(sort_order(&rows, LinkColumn::Link, true), [1, 4, 0, 3, 2]);
        // One folder for all: by name.
        assert_eq!(sort_order(&rows, LinkColumn::Folder, true), [1, 3, 2, 0, 4]);
    }

    #[test]
    fn finds_links_in_text_and_html() {
        let text = "Get https://a.com/x.zip, or (http://b.org/y.iso). <a href=\"https://c.net/z?a=1&amp;b=2\">z</a> \
                    again https://a.com/x.zip and ftp://d.io/f and https://en.wikipedia.org/wiki/Foo_(bar).";
        assert_eq!(
            find(text),
            vec![
                "https://a.com/x.zip",
                "http://b.org/y.iso",
                "https://c.net/z?a=1&b=2",
                "ftp://d.io/f",
                "https://en.wikipedia.org/wiki/Foo_(bar)",
            ]
        );
        assert!(find("no links, just http:// and words").is_empty());
    }

    #[test]
    fn reads_a_paste_as_jdownloader_does() {
        // Masked, run together, without a scheme, and a link inside a redirect's query.
        assert_eq!(find("see hxxps://a.com/x.zip and h**p://b.org/y.iso"), vec!["https://a.com/x.zip", "http://b.org/y.iso"]);
        assert_eq!(find("https://a.com/1.ziphttps://a.com/2.zip"), vec!["https://a.com/1.zip", "https://a.com/2.zip"]);
        assert_eq!(
            find("https://r.example.com/go?u=https%3A%2F%2Fcdn.example.com%2Ff.zip&x=1"),
            vec!["https://r.example.com/go?u=https%3A%2F%2Fcdn.example.com%2Ff.zip&x=1", "https://cdn.example.com/f.zip"]
        );
        assert_eq!(find("Get it at www.example.com/files/a.zip, today."), vec!["http://www.example.com/files/a.zip"]);
        assert_eq!(find("example.com/files/a.zip"), vec!["http://example.com/files/a.zip"]);
        assert_eq!(find("  192.168.1.5:8080/a.iso\n"), vec!["http://192.168.1.5:8080/a.iso"]);
        // Not links: a scheme inside a word, a path alone, a file name, words.
        assert!(find("look at notes.txt and src/main.rs").is_empty());
        assert!(find("files/a.zip").is_empty());
        assert!(find("v1.2/notes").is_empty());
        assert!(find("swww.example.com").is_empty());
        // A link already written with its scheme isn't found twice.
        assert_eq!(find("https://www.example.com/a.zip"), vec!["https://www.example.com/a.zip"]);
        // "http" inside a query value isn't split off.
        assert_eq!(find("https://a.com/?next=http://b.com/"), vec!["https://a.com/?next=http://b.com/", "http://b.com/"]);
    }

    #[test]
    fn copied_html_links_count_against_their_page() {
        let html = r##"<a href="files/a.zip">A</a> <a href='/b.iso'>B</a> <a href=c.pdf>C</a> <a href="#top">T</a> <a href="https://x.org/d.zip">D</a>"##;
        let text = absolute_html(html, "https://example.com/dl/index.html");
        assert_eq!(
            find(&text),
            vec!["https://example.com/dl/files/a.zip", "https://example.com/b.iso", "https://example.com/dl/c.pdf", "https://x.org/d.zip"]
        );
    }

    #[test]
    fn copied_from_reads_utf8_and_utf16() {
        assert_eq!(copied_from(b"https://example.com/dl/index.html").as_deref(), Some("https://example.com/dl/index.html"));
        let wide: Vec<u8> = "https://example.com/a\nPage".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(copied_from(&wide).as_deref(), Some("https://example.com/a"));
        assert_eq!(copied_from(b"file:///etc/passwd"), None);
        assert_eq!(copied_from(b""), None);
    }

    #[test]
    fn expands_batch_ranges() {
        let l = expand_batch("https://e.com/img^(###1-100).jpg").unwrap();
        assert_eq!((l.len(), l[0].as_str(), l[99].as_str()), (100, "https://e.com/img001.jpg", "https://e.com/img100.jpg"));
        assert_eq!(expand_batch("https://e.com/^(8-bH)").unwrap(), ["https://e.com/8", "https://e.com/9", "https://e.com/a", "https://e.com/b"]);
        assert_eq!(expand_batch("https://e.com/^(##0E-10h)").unwrap(), ["https://e.com/0E", "https://e.com/0F", "https://e.com/10"]);
        assert_eq!(expand_batch("https://e.com/^(a-c)").unwrap().len(), 3);
        let az = expand_batch("https://e.com/^(AA-BZ)").unwrap();
        assert_eq!((az.len(), az[0].as_str(), az[51].as_str()), (52, "https://e.com/AA", "https://e.com/BZ"));
        assert_eq!(expand_batch("https://e.com/^(1-2,7-8)").unwrap().len(), 4);
        assert_eq!(expand_batch("https://e.com/^(1-2)/^(a-b)").unwrap()[3], "https://e.com/2/b");
        assert_eq!(expand_batch("https://e.com/^(3-1)").unwrap()[0], "https://e.com/3");
        assert!(expand_batch("https://e.com/x").is_err());
        assert!(expand_batch("https://e.com/^(1-x)").is_err());
        assert!(expand_batch("https://e.com/^(1-200000)").is_err());
    }
}
