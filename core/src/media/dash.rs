//! DASH manifests (`.mpd`, ISO/IEC 23009-1): a video's qualities and its sound, as separate tracks of MP4 fragments.
//! Read into the same pieces HLS gives (an init section and the pieces after it), so the download and the joiner
//! treat both alike. Three ways of addressing the pieces:
//! - `SegmentBase`: one file per quality, its init section and index (`sidx`) at byte ranges (Facebook's reels);
//! - `SegmentList`: each piece's address or byte range listed;
//! - `SegmentTemplate`: addresses made from a pattern (`$Number$`, `$Time$`), by duration or a `SegmentTimeline`.

use reqwest::Url;

/// How a quality's pieces are found.
#[derive(Clone, Debug, PartialEq)]
pub enum Addressing {
    /// One file: its init section's and index's byte ranges (inclusive), when given.
    Base { url: Url, init: Option<(u64, u64)>, index: Option<(u64, u64)> },
    /// The init section and the pieces, each an address and maybe an inclusive byte range.
    Pieces { init: Option<(Url, Option<(u64, u64)>)>, media: Vec<(Url, Option<(u64, u64)>)> },
}

/// One quality of one track.
#[derive(Clone, Debug, PartialEq)]
pub struct Representation {
    pub id: String,
    pub bandwidth: u64,
    pub width: u32,
    pub height: u32,
    pub codecs: String,
    pub addressing: Addressing,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Manifest {
    /// Seconds, when known.
    pub duration: Option<f64>,
    pub video: Vec<Representation>,
    pub audio: Vec<Representation>,
    /// Some track is protected (DRM: ContentProtection).
    pub protected: bool,
    /// A live presentation (`type="dynamic"`).
    pub live: bool,
}

/// "PT1H2M3.5S" → seconds.
pub fn iso_duration(text: &str) -> Option<f64> {
    let t = text.trim().strip_prefix('P')?;
    let (days, time) = match t.split_once('T') {
        Some((d, rest)) => (d, rest),
        None => (t, ""),
    };
    let mut secs = 0.0;
    if let Some(d) = days.strip_suffix('D') {
        secs += d.parse::<f64>().ok()? * 86_400.0;
    }
    let mut number = String::new();
    for c in time.chars() {
        match c {
            'H' | 'M' | 'S' => {
                let v: f64 = number.parse().ok()?;
                secs += v * match c {
                    'H' => 3600.0,
                    'M' => 60.0,
                    _ => 1.0,
                };
                number.clear();
            }
            _ => number.push(c),
        }
    }
    Some(secs)
}

fn range(text: &str) -> Option<(u64, u64)> {
    let (a, b) = text.split_once('-')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

fn children<'a, 'i>(node: roxmltree::Node<'a, 'i>, name: &'a str) -> impl Iterator<Item = roxmltree::Node<'a, 'i>> + 'a {
    node.children().filter(move |c| c.is_element() && c.tag_name().name() == name)
}

fn child<'a, 'i>(node: roxmltree::Node<'a, 'i>, name: &'a str) -> Option<roxmltree::Node<'a, 'i>> {
    children(node, name).next()
}

/// The base address at this level: its BaseURL joined onto the one above.
fn base_of(node: roxmltree::Node, above: &Url) -> Url {
    child(node, "BaseURL").and_then(|b| b.text()).and_then(|t| above.join(t.trim()).ok()).unwrap_or_else(|| above.clone())
}

/// A template's address: `$RepresentationID$`, `$Number$`, `$Number%05d$`, `$Bandwidth$`, `$Time$`, `$$`.
fn fill(template: &str, id: &str, bandwidth: u64, number: u64, time: u64) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('$') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('$') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let name = &after[..end];
        let (key, format) = name.split_once('%').map(|(k, f)| (k, Some(f))).unwrap_or((name, None));
        let value = match key {
            "" => "$".to_string(),
            "RepresentationID" => id.to_string(),
            "Number" | "Bandwidth" | "Time" => {
                let n = match key {
                    "Number" => number,
                    "Bandwidth" => bandwidth,
                    _ => time,
                };
                match format.and_then(|f| f.strip_suffix('d')).and_then(|w| w.trim_start_matches('0').parse::<usize>().ok()) {
                    Some(width) => format!("{n:0width$}"),
                    None => n.to_string(),
                }
            }
            _ => format!("${name}$"),
        };
        out.push_str(&value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// An attribute of the nearest of these elements that has it (a Representation's own, else its AdaptationSet's).
fn inherited<'a>(nodes: &[Option<roxmltree::Node<'a, 'a>>], name: &str) -> Option<&'a str> {
    nodes.iter().flatten().find_map(|n| n.attribute(name))
}

/// Reads a manifest fetched from `base` (relative addresses are resolved against it).
pub fn parse(text: &str, base: &Url) -> Result<Manifest, String> {
    let doc = roxmltree::Document::parse(text).map_err(|_| "The DASH manifest can't be read".to_string())?;
    let mpd = doc.root_element();
    if mpd.tag_name().name() != "MPD" {
        return Err("This isn't a DASH manifest".into());
    }
    let mut m = Manifest { live: mpd.attribute("type") == Some("dynamic"), ..Default::default() };
    m.duration = mpd.attribute("mediaPresentationDuration").and_then(iso_duration);
    let mpd_base = base_of(mpd, base);
    // The first period: a presentation of several (adverts between parts) is read as its main one.
    let Some(period) = child(mpd, "Period") else { return Err("The DASH manifest has no content".into()) };
    let period_base = base_of(period, &mpd_base);
    let period_seconds = period.attribute("duration").and_then(iso_duration).or(m.duration);
    for set in children(period, "AdaptationSet") {
        let set_base = base_of(set, &period_base);
        let kind = set
            .attribute("contentType")
            .or_else(|| set.attribute("mimeType").and_then(|t| t.split('/').next()))
            .or_else(|| children(set, "Representation").find_map(|r| r.attribute("mimeType")).and_then(|t| t.split('/').next()))
            .unwrap_or("");
        if kind != "video" && kind != "audio" {
            continue; // subtitles, pictures
        }
        let set_protected = child(set, "ContentProtection").is_some();
        for rep in children(set, "Representation") {
            if set_protected || child(rep, "ContentProtection").is_some() {
                m.protected = true;
                continue;
            }
            let rep_base = base_of(rep, &set_base);
            let id = rep.attribute("id").unwrap_or("").to_string();
            let bandwidth = rep.attribute("bandwidth").and_then(|b| b.parse().ok()).unwrap_or(0);
            let levels = [Some(rep), Some(set), Some(period)];
            let template = levels.iter().flatten().find_map(|n| child(*n, "SegmentTemplate"));
            let list = levels.iter().flatten().find_map(|n| child(*n, "SegmentList"));
            let segment_base = levels.iter().flatten().find_map(|n| child(*n, "SegmentBase"));
            let addressing = if template.is_some() {
                // A template at a lower level overrides the attributes above it.
                let templates: Vec<Option<roxmltree::Node>> = levels.iter().map(|l| l.and_then(|n| child(n, "SegmentTemplate"))).collect();
                let attr = |name: &str| inherited(&templates, name);
                let timescale: u64 = attr("timescale").and_then(|v| v.parse().ok()).unwrap_or(1);
                let start: u64 = attr("startNumber").and_then(|v| v.parse().ok()).unwrap_or(1);
                let media = attr("media").ok_or("A DASH template has no address pattern")?;
                let init = attr("initialization").and_then(|i| rep_base.join(&fill(i, &id, bandwidth, 0, 0)).ok()).map(|u| (u, None));
                let mut pieces = Vec::new();
                if let Some(timeline) = templates.iter().flatten().find_map(|n| child(*n, "SegmentTimeline")) {
                    let mut time = 0u64;
                    let mut number = start;
                    let entries: Vec<roxmltree::Node> = children(timeline, "S").collect();
                    for (i, s) in entries.iter().enumerate() {
                        if let Some(t) = s.attribute("t").and_then(|v| v.parse().ok()) {
                            time = t;
                        }
                        let d: u64 = s.attribute("d").and_then(|v| v.parse().ok()).filter(|&d| d > 0).ok_or("A DASH timeline entry has no length")?;
                        let r: i64 = s.attribute("r").and_then(|v| v.parse().ok()).unwrap_or(0);
                        // r = -1: repeat until the next entry, or the end of the period.
                        let repeats = if r >= 0 {
                            r as u64
                        } else {
                            let until = entries.get(i + 1).and_then(|n| n.attribute("t")).and_then(|v| v.parse::<u64>().ok()).or_else(|| period_seconds.map(|p| (p * timescale as f64) as u64)).unwrap_or(time + d);
                            (until.saturating_sub(time)).div_ceil(d).saturating_sub(1)
                        };
                        for _ in 0..=repeats {
                            pieces.push((rep_base.join(&fill(media, &id, bandwidth, number, time)).map_err(|e| e.to_string())?, None));
                            time += d;
                            number += 1;
                        }
                    }
                } else {
                    let d: u64 = attr("duration").and_then(|v| v.parse().ok()).filter(|&d| d > 0).ok_or("A DASH template has neither a timeline nor a piece length")?;
                    let seconds = period_seconds.ok_or("The DASH manifest doesn't say how long it is")?;
                    let count = ((seconds * timescale as f64) / d as f64).ceil() as u64;
                    for k in 0..count {
                        pieces.push((rep_base.join(&fill(media, &id, bandwidth, start + k, k * d)).map_err(|e| e.to_string())?, None));
                    }
                }
                Addressing::Pieces { init, media: pieces }
            } else if let Some(l) = list {
                let init = child(l, "Initialization").map(|i| {
                    let url = i.attribute("sourceURL").and_then(|s| rep_base.join(s).ok()).unwrap_or_else(|| rep_base.clone());
                    (url, i.attribute("range").and_then(range))
                });
                let media = children(l, "SegmentURL")
                    .map(|s| {
                        let url = s.attribute("media").and_then(|m| rep_base.join(m).ok()).unwrap_or_else(|| rep_base.clone());
                        (url, s.attribute("mediaRange").and_then(range))
                    })
                    .collect();
                Addressing::Pieces { init, media }
            } else {
                let init = segment_base.and_then(|b| child(b, "Initialization")).and_then(|i| i.attribute("range")).and_then(range);
                let index = segment_base.and_then(|b| b.attribute("indexRange")).and_then(range);
                Addressing::Base { url: rep_base.clone(), init, index }
            };
            let r = Representation {
                id,
                bandwidth,
                width: rep.attribute("width").or(set.attribute("width")).and_then(|v| v.parse().ok()).unwrap_or(0),
                height: rep.attribute("height").or(set.attribute("height")).and_then(|v| v.parse().ok()).unwrap_or(0),
                codecs: rep.attribute("codecs").or(set.attribute("codecs")).unwrap_or("").to_string(),
                addressing,
            };
            if kind == "video" { m.video.push(r) } else { m.audio.push(r) }
        }
    }
    if m.video.is_empty() && m.audio.is_empty() {
        return Err(if m.protected { "This stream is protected (DRM): it can't be saved".into() } else { "The DASH manifest lists nothing to download".into() });
    }
    Ok(m)
}

impl Manifest {
    /// The best picture: the most pixels, then the most bits a second.
    pub fn best_video(&self) -> Option<&Representation> {
        self.video.iter().max_by_key(|r| (r.width as u64 * r.height as u64, r.bandwidth))
    }

    /// The best sound: the most bits a second.
    pub fn best_audio(&self) -> Option<&Representation> {
        self.audio.iter().max_by_key(|r| r.bandwidth)
    }
}

/// A segment index (`sidx` box, starting at byte `at` of its file): each subsegment's byte range (inclusive).
pub fn sidx_ranges(data: &[u8], at: u64) -> Option<Vec<(u64, u64)>> {
    let mut i = 0;
    while i + 8 <= data.len() {
        let size = u32::from_be_bytes(data[i..i + 4].try_into().ok()?) as usize;
        if &data[i + 4..i + 8] == b"sidx" {
            let b = data.get(i + 8..i + size)?;
            let version = b[0];
            let (first_offset, rest) = if version == 0 {
                // Version 0: after version and flags (0..4), reference id (4..8), timescale (8..12) and earliest
                // presentation time (12..16), the first offset (16..20).
                (u32::from_be_bytes(b.get(16..20)?.try_into().ok()?) as u64, 20)
            } else {
                // Version 1: 64-bit earliest presentation time (12..20) and first offset (20..28).
                (u64::from_be_bytes(b.get(20..28)?.try_into().ok()?), 28)
            };
            let count = u16::from_be_bytes(b.get(rest + 2..rest + 4)?.try_into().ok()?) as usize;
            let mut offset = at + (i + size) as u64 + first_offset;
            let mut out = Vec::with_capacity(count);
            for k in 0..count {
                let e = b.get(rest + 4 + 12 * k..rest + 16 + 12 * k)?;
                let word = u32::from_be_bytes(e[0..4].try_into().ok()?);
                // A reference to another index (hierarchical) isn't one of the media's pieces.
                if word >> 31 == 1 {
                    return None;
                }
                let len = (word & 0x7fff_ffff) as u64;
                out.push((offset, offset + len - 1));
                offset += len;
            }
            return Some(out);
        }
        if size < 8 {
            return None;
        }
        i += size;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_and_templates() {
        assert_eq!(iso_duration("PT127.370911S"), Some(127.370911));
        assert_eq!(iso_duration("PT1H2M3S"), Some(3723.0));
        assert_eq!(fill("chunk-$RepresentationID$-$Number%05d$.m4s", "0", 1, 7, 0), "chunk-0-00007.m4s");
        assert_eq!(fill("v/$Bandwidth$/$Time$.mp4$$", "x", 800, 1, 9000), "v/800/9000.mp4$");
    }

    #[test]
    fn facebooks_layout_with_drm_and_live_told_apart() {
        let base = Url::parse("https://www.facebook.com/reel/1").unwrap();
        let fb = r#"<?xml version="1.0"?><MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT127.370911S">
            <Period id="0"><AdaptationSet contentType="video">
            <Representation id="360" bandwidth="669765" codecs="avc1.4d001e" mimeType="video/mp4" width="360" height="640"><BaseURL>https://cdn.example.com/a.mp4?x=1&amp;y=2</BaseURL><SegmentBase indexRange="887-1230"><Initialization range="0-886"/></SegmentBase></Representation>
            <Representation id="720" bandwidth="2991469" width="720" height="1280"><BaseURL>https://cdn.example.com/b.mp4</BaseURL><SegmentBase indexRange="892-1235"><Initialization range="0-891"/></SegmentBase></Representation>
            </AdaptationSet><AdaptationSet contentType="audio"><Representation id="a" bandwidth="80450" mimeType="audio/mp4"><BaseURL>https://cdn.example.com/s.mp4</BaseURL><SegmentBase indexRange="824-1623"><Initialization range="0-823"/></SegmentBase></Representation></AdaptationSet></Period></MPD>"#;
        let m = parse(fb, &base).unwrap();
        assert_eq!(m.duration, Some(127.370911));
        let v = m.best_video().unwrap();
        assert_eq!((v.id.as_str(), v.height), ("720", 1280));
        assert_eq!(v.addressing, Addressing::Base { url: Url::parse("https://cdn.example.com/b.mp4").unwrap(), init: Some((0, 891)), index: Some((892, 1235)) });
        let first = &m.video[0].addressing;
        assert!(matches!(first, Addressing::Base { url, .. } if url.as_str() == "https://cdn.example.com/a.mp4?x=1&y=2"), "entities in addresses are read");
        assert_eq!(m.best_audio().unwrap().id, "a");
        let drm = fb.replace("<AdaptationSet contentType=\"video\">", "<AdaptationSet contentType=\"video\"><ContentProtection schemeIdUri=\"urn:uuid:edef8ba9\"/>");
        let m = parse(&drm, &base).unwrap();
        assert!(m.protected && m.video.is_empty());
        assert!(parse(&fb.replace("type=\"static\"", "type=\"dynamic\""), &base).unwrap().live);
    }
}
