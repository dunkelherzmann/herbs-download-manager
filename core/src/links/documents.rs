//! The text of word-processor files, web archives and bookmarks, with their hyperlinks' targets added (§5.7), as the
//! Mac reads them through AppKit: `.docx`, `.odt`, `.doc` (Word 97–2003), `.webloc` and `.webarchive`. Each gives
//! `None` when the file isn't what its name says, so the caller says it has no links.

use std::io::Read;

/// As much as one part of a document may unpack to: far more than any list of links, and a zip bomb stops there.
const MAX_PART: u64 = 50 * 1024 * 1024;

/// One file inside a zip (a .docx's or .odt's part), or `None` when there is no such part.
fn part(bytes: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).ok()?;
    let entry = archive.by_name(name).ok()?;
    let mut out = Vec::new();
    entry.take(MAX_PART).read_to_end(&mut out).ok()?;
    Some(out)
}

/// The text of an XML part: `text` elements' text, a line break before each `paragraph` element, a space for each
/// `space` element; plus each attribute value `link` picks out, one per line at the end.
fn xml_text(xml: &[u8], text: &[&str], paragraph: &[&str], space: &[&str], link: impl Fn(roxmltree::Node) -> Option<String>) -> Option<String> {
    let xml = std::str::from_utf8(xml).ok()?;
    let doc = roxmltree::Document::parse_with_options(xml, roxmltree::ParsingOptions { allow_dtd: false, ..Default::default() }).ok()?;
    let (mut out, mut links) = (String::new(), Vec::new());
    for node in doc.descendants() {
        if node.is_element() {
            let name = node.tag_name().name();
            if paragraph.contains(&name) && !out.is_empty() {
                out.push('\n');
            } else if space.contains(&name) {
                out.push(' ');
            }
            links.extend(link(node));
        } else if node.is_text() && node.parent_element().is_some_and(|p| text.contains(&p.tag_name().name())) {
            out.push_str(node.text().unwrap_or_default());
        }
    }
    for l in links {
        out.push('\n');
        out.push_str(&l);
    }
    Some(out)
}

/// A Word document (.docx): its text, its field codes (HYPERLINK "…") and its hyperlinks' targets.
pub fn docx(bytes: &[u8]) -> Option<String> {
    let body = part(bytes, "word/document.xml")?;
    let mut text = xml_text(&body, &["t", "instrText"], &["p"], &["tab", "br"], |_| None)?;
    if let Some(rels) = part(bytes, "word/_rels/document.xml.rels") {
        let targets = xml_text(&rels, &[], &[], &[], |n| {
            (n.tag_name().name() == "Relationship" && n.attribute("Type").is_some_and(|t| t.ends_with("/hyperlink")))
                .then(|| n.attribute("Target").map(str::to_string))
                .flatten()
        })?;
        text.push_str(&targets);
    }
    Some(text)
}

/// An OpenDocument text (.odt): its text and its links' targets.
pub fn odt(bytes: &[u8]) -> Option<String> {
    let content = part(bytes, "content.xml")?;
    xml_text(&content, &["p", "h", "span", "a", "s"], &["p", "h"], &["tab", "s", "line-break"], |n| {
        n.attribute(("http://www.w3.org/1999/xlink", "href")).map(str::to_string)
    })
}

/// A Word 97–2003 document (.doc, an OLE compound file): its runs of text, stored as single bytes or as UTF-16, field
/// codes (HYPERLINK "…") included. Only the text matters here (the links in it), so the rest of the file's bytes,
/// read the same way, add nothing a link could be found in. A ".doc" that isn't one (RTF or HTML saved with that
/// name, as Word often did) is read as text.
pub fn doc(bytes: &[u8]) -> Option<String> {
    if !bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) {
        return super::plain_text(bytes);
    }
    let printable = |c: u16| (0x20..0x7F).contains(&c) || c >= 0xA0 && c != 0xFFFF;
    // Each run of text with where it starts, so the links come in the file's order.
    let mut runs: Vec<(usize, String)> = Vec::new();
    let mut keep = |start: usize, run: &mut String| {
        if run.chars().count() >= 4 {
            runs.push((start, std::mem::take(run)));
        }
        run.clear();
    };
    let (mut run, mut start) = (String::new(), 0);
    for (i, &b) in bytes.iter().enumerate() {
        if b < 0x80 && printable(b as u16) {
            if run.is_empty() {
                start = i;
            }
            run.push(b as char);
        } else {
            keep(start, &mut run);
        }
    }
    keep(start, &mut run);
    for offset in 0..2 {
        for (i, pair) in bytes[offset..].as_chunks::<2>().0.iter().enumerate() {
            let unit = u16::from_le_bytes(*pair);
            match char::from_u32(unit as u32) {
                Some(c) if printable(unit) => {
                    if run.is_empty() {
                        start = offset + 2 * i;
                    }
                    run.push(c);
                }
                _ => keep(start, &mut run),
            }
        }
        keep(start, &mut run);
    }
    runs.sort_by_key(|(start, _)| *start);
    Some(runs.into_iter().map(|(_, run)| run + "\n").collect())
}

/// A Safari bookmark (.webloc, an XML or binary property list): its address.
pub fn webloc(bytes: &[u8]) -> Option<String> {
    let value = plist::Value::from_reader(std::io::Cursor::new(bytes)).ok()?;
    value.as_dictionary()?.get("URL")?.as_string().map(str::to_string)
}

/// A Safari web archive (.webarchive, a binary property list): its page's HTML and its frames', as text.
pub fn webarchive(bytes: &[u8]) -> Option<String> {
    fn page(archive: &plist::Dictionary, out: &mut String) {
        let main = archive.get("WebMainResource").and_then(|r| r.as_dictionary());
        if let Some(data) = main.and_then(|r| r.get("WebResourceData")).and_then(|d| d.as_data())
            && let Some(mut text) = super::plain_text(data)
        {
            // The doctype's address (w3.org's DTD) isn't one of the page's links, as the Mac reads it.
            if let Some(at) = text.to_ascii_lowercase().find("<!doctype")
                && let Some(len) = text[at..].find('>')
            {
                text.replace_range(at..=at + len, "");
            }
            out.push_str(&text);
            out.push('\n');
        }
        for frame in archive.get("WebSubframeArchives").and_then(|f| f.as_array()).into_iter().flatten() {
            if let Some(frame) = frame.as_dictionary() {
                page(frame, out);
            }
        }
    }
    let value = plist::Value::from_reader(std::io::Cursor::new(bytes)).ok()?;
    let mut out = String::new();
    page(value.as_dictionary()?, &mut out);
    Some(out)
}
