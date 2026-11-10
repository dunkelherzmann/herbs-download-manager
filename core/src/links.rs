//! Finding links in pasted or dropped text (SPECIFICATION.md §5.2–§5.4) and expanding batch patterns (§5.5).

/// The text of a dropped or imported file (§5.4, §5.7) for the hosts that read it themselves (Linux, Windows):
/// plain text, HTML, rich text (its links are plain text inside it) and the like, UTF-8 or UTF-16 (by its byte-order
/// mark). `None` for a file that isn't text (a NUL in its first 8 KB: a zip, an image) or is over 50 MB.
pub fn text_of_file(path: &std::path::Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > 50 * 1024 * 1024 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let utf16 = |le: bool| {
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| if le { u16::from_le_bytes(c) } else { u16::from_be_bytes(c) })
            .collect();
        Some(String::from_utf16_lossy(&units))
    };
    match bytes.as_slice() {
        [0xFF, 0xFE, ..] => utf16(true),
        [0xFE, 0xFF, ..] => utf16(false),
        _ if bytes.iter().take(8192).any(|&b| b == 0) => None,
        _ => Some(String::from_utf8_lossy(&bytes).into_owned()),
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
/// belongs to the sentence (".", ",", ")", quotes) is left out.
pub fn find(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let lower = text.to_ascii_lowercase();
    let mut i = 0;
    while i < text.len() {
        let rest = &lower[i..];
        let Some(at) = ["https://", "http://", "ftp://"].iter().filter_map(|p| rest.find(p)).min() else { break };
        let start = i + at;
        let end = text[start..]
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`' | '[' | ']' | '{' | '}' | '|' | '\\'))
            .map_or(text.len(), |n| start + n);
        let mut link = &text[start..end];
        while let Some(last) = link.chars().last() {
            let unbalanced_paren = last == ')' && link.matches('(').count() < link.matches(')').count();
            if matches!(last, '.' | ',' | ';' | ':' | '!' | '?') || unbalanced_paren {
                link = &link[..link.len() - last.len_utf8()];
            } else {
                break;
            }
        }
        let link = link.replace("&amp;", "&");
        if reqwest::Url::parse(&link).is_ok_and(|u| u.host_str().is_some()) && !out.contains(&link) {
            out.push(link);
        }
        i = end.max(start + 1);
    }
    out
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
