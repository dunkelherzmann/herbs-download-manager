//! Import Links reads word-processor files, web archives and bookmarks on Linux and Windows as the Mac does (§5.7):
//! their text and their hyperlinks' targets. Each fixture is made here, the way its app writes it.

use std::io::Write;
use std::path::{Path, PathBuf};

use hd_core::links::{find, text_of_file};

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hd-import-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn zip_file(path: &Path, parts: &[(&str, &str)]) {
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, text) in parts {
        zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

fn links_in(path: &Path) -> Vec<String> {
    find(&text_of_file(path).unwrap_or_else(|| panic!("no text read from {}", path.display())))
}

#[test]
fn a_word_document_gives_its_text_its_field_codes_and_its_hyperlinks() {
    let dir = folder("docx");
    let body = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"
            xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <w:body>
    <w:p><w:r><w:t xml:space="preserve">Get it from https://example.com/</w:t></w:r><w:r><w:t>split/run.zip</w:t></w:r></w:p>
    <w:p><w:hyperlink r:id="rId5"><w:r><w:t>the mirror</w:t></w:r></w:hyperlink></w:p>
    <w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> HYPERLINK "https://field.example.org/c.iso" </w:instrText></w:r>
      <w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>shown text</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>
  </w:body>
</w:document>"#;
    let rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
  <Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://mirror.example.net/b.zip" TargetMode="External"/>
</Relationships>"#;
    let path = dir.join("Links.docx");
    zip_file(&path, &[("[Content_Types].xml", "<Types/>"), ("word/document.xml", body), ("word/_rels/document.xml.rels", rels)]);
    assert_eq!(links_in(&path), [
        "https://example.com/split/run.zip",
        "https://field.example.org/c.iso",
        "https://mirror.example.net/b.zip",
    ]);
}

#[test]
fn an_opendocument_text_gives_its_text_and_its_links() {
    let dir = folder("odt");
    let content = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
    xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:xlink="http://www.w3.org/1999/xlink">
  <office:body><office:text>
    <text:h>Files</text:h>
    <text:p>First: https://example.com/a.zip<text:line-break/>then <text:span>https://example.com/b.zip</text:span></text:p>
    <text:p><text:a xlink:type="simple" xlink:href="https://linked.example.org/c.iso">the third file</text:a></text:p>
  </office:text></office:body>
</office:document-content>"#;
    let path = dir.join("Links.odt");
    zip_file(&path, &[("mimetype", "application/vnd.oasis.opendocument.text"), ("content.xml", content)]);
    assert_eq!(links_in(&path), [
        "https://example.com/a.zip",
        "https://example.com/b.zip",
        "https://linked.example.org/c.iso",
    ]);
}

#[test]
fn a_word_97_document_gives_the_links_in_its_text() {
    let dir = folder("doc");
    // An OLE compound file's signature, then text as Word stores it: single bytes, and UTF-16 at either alignment.
    let mut bytes = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0, 0x3E, 0, 3, 0, 0xFE, 0xFF];
    bytes.extend(b"\x13 HYPERLINK \"https://example.com/a.zip\" \x14here\x15\0\0\x01");
    bytes.extend("see https://example.com/b.zip".encode_utf16().flat_map(|u| u.to_le_bytes()));
    bytes.extend([0, 0, 0]);
    bytes.extend("and https://example.com/c.zip".encode_utf16().flat_map(|u| u.to_le_bytes()));
    bytes.extend([0; 16]);
    let path = dir.join("Links.doc");
    std::fs::write(&path, &bytes).unwrap();
    assert_eq!(links_in(&path), ["https://example.com/a.zip", "https://example.com/b.zip", "https://example.com/c.zip"]);
    // A ".doc" that is really rich text, as Word often saved them, is read as text.
    let rtf = dir.join("Rich.doc");
    std::fs::write(&rtf, r#"{\rtf1 {\field{\*\fldinst{HYPERLINK "https://example.com/d.zip"}}{\fldrslt here}}}"#).unwrap();
    assert_eq!(links_in(&rtf), ["https://example.com/d.zip"]);
}

#[test]
fn rich_text_and_rtfd_give_their_hyperlinks() {
    let dir = folder("rtf");
    let rtf = r#"{\rtf1\ansi {\field{\*\fldinst{HYPERLINK "https://example.com/a.zip"}}{\fldrslt the file}}\par
https://example.com/b.zip\par}"#;
    std::fs::write(dir.join("Links.rtf"), rtf).unwrap();
    assert_eq!(links_in(&dir.join("Links.rtf")), ["https://example.com/a.zip", "https://example.com/b.zip"]);
    let rtfd = dir.join("Links.rtfd");
    std::fs::create_dir(&rtfd).unwrap();
    std::fs::write(rtfd.join("TXT.rtf"), rtf).unwrap();
    assert_eq!(links_in(&rtfd), ["https://example.com/a.zip", "https://example.com/b.zip"]);
    // Any other folder has no text.
    assert_eq!(text_of_file(&dir), None);
}

#[test]
fn bookmarks_and_web_archives_give_their_addresses() {
    let dir = folder("plist");
    let mut bookmark = plist::Dictionary::new();
    bookmark.insert("URL".into(), "https://example.com/bookmarked.zip".into());
    let binary = dir.join("Binary.webloc");
    plist::Value::Dictionary(bookmark.clone()).to_file_binary(&binary).unwrap();
    assert_eq!(links_in(&binary), ["https://example.com/bookmarked.zip"]);
    let xml = dir.join("Xml.webloc");
    plist::Value::Dictionary(bookmark).to_file_xml(&xml).unwrap();
    assert_eq!(links_in(&xml), ["https://example.com/bookmarked.zip"]);

    let resource = |html: &str| {
        let mut r = plist::Dictionary::new();
        r.insert("WebResourceData".into(), plist::Value::Data(html.as_bytes().to_vec()));
        r.insert("WebResourceMIMEType".into(), "text/html".into());
        r.insert("WebResourceURL".into(), "https://example.com/page.html".into());
        r
    };
    let mut frame = plist::Dictionary::new();
    frame.insert("WebMainResource".into(), resource(r#"<a href="https://example.com/in-frame.iso">x</a>"#).into());
    let mut archive = plist::Dictionary::new();
    archive.insert("WebMainResource".into(), resource(r#"<!DOCTYPE html PUBLIC "-//W3C//DTD HTML 4.01//EN" "http://www.w3.org/TR/html4/strict.dtd"><p><a href="https://example.com/a.zip?x=1&amp;y=2">a</a></p>"#).into());
    archive.insert("WebSubframeArchives".into(), plist::Value::Array(vec![frame.into()]));
    let path = dir.join("Page.webarchive");
    plist::Value::Dictionary(archive).to_file_binary(&path).unwrap();
    assert_eq!(links_in(&path), ["https://example.com/a.zip?x=1&y=2", "https://example.com/in-frame.iso"]);
}

#[test]
fn files_that_arent_what_their_names_say_have_no_text() {
    let dir = folder("broken");
    for name in ["Broken.docx", "Broken.odt", "Broken.webloc", "Broken.webarchive"] {
        std::fs::write(dir.join(name), b"not really one https://example.com/x.zip").unwrap();
        assert_eq!(text_of_file(&dir.join(name)), None, "{name}");
    }
    // A zip without a Word document inside.
    zip_file(&dir.join("Other.docx"), &[("readme.txt", "https://example.com/y.zip")]);
    assert_eq!(text_of_file(&dir.join("Other.docx")), None);
}
