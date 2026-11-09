import AppKit
import HDCore
import UniformTypeIdentifiers

/// Finds links in files (SPECIFICATION.md §5.7): plain text with or without an extension (.txt, .md, .csv, .json,
/// .xml, …), HTML, rich text and word-processor files (.rtf, .rtfd, .doc, .docx, .odt, web archives, whose
/// hyperlinks count too), and .webloc bookmarks. Files that aren't text (a zip, an image) give no links.
enum LinkReader {
    /// Big enough for any list of links, small enough not to read a disk image by mistake.
    static let maxBytes = 50_000_000

    static func links(fromFile url: URL) -> [String] {
        findLinks(text: text(ofFile: url) ?? "")
    }

    /// The file's text with its hyperlinks' targets added, or nil if it isn't a text-based file.
    static func text(ofFile url: URL) -> String? {
        let values = try? url.resourceValues(forKeys: [.fileSizeKey, .isDirectoryKey, .contentTypeKey])
        let type = values?.contentType ?? UTType(filenameExtension: url.pathExtension)
        // An .rtfd is a folder; any other folder has no text.
        if let type, isDocument(type) { return document(url) }
        guard values?.isDirectory != true, (values?.fileSize ?? 0) <= maxBytes,
              let data = try? Data(contentsOf: url, options: .mappedIfSafe) else { return nil }
        if url.pathExtension.lowercased() == "webloc",
           let plist = try? PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any] {
            return plist["URL"] as? String
        }
        return plainText(data)
    }

    /// Rich text and word-processor files, read by AppKit; their hyperlinks are added to the text.
    private static func isDocument(_ type: UTType) -> Bool {
        [UTType.rtf, .rtfd, .flatRTFD].contains { type.conforms(to: $0) }
            || ["com.microsoft.word.doc", "org.openxmlformats.wordprocessingml.document",
                "org.oasis-open.opendocument.text", "com.apple.webarchive"].contains(type.identifier)
    }

    private static func document(_ url: URL) -> String? {
        guard let doc = try? NSAttributedString(url: url, options: [:], documentAttributes: nil) else { return nil }
        var text = doc.string
        doc.enumerateAttribute(.link, in: NSRange(location: 0, length: doc.length)) { value, _, _ in
            if let link = value as? URL { text += "\n" + link.absoluteString }
            if let link = value as? String { text += "\n" + link }
        }
        return text
    }

    /// UTF-8 or UTF-16 text; nil when the start of the file holds a NUL byte (a binary file).
    private static func plainText(_ data: Data) -> String? {
        if data.starts(with: [0xFF, 0xFE]) || data.starts(with: [0xFE, 0xFF]) {
            return String(data: data, encoding: .utf16)
        }
        if data.prefix(8192).contains(0) { return nil }
        return String(decoding: data, as: UTF8.self)
    }
}
