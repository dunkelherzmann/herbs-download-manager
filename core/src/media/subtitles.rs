//! A stream's subtitles (SPECIFICATION.md §15): WebVTT files read into cues, and the cues written as an MP4 subtitle
//! track (`tx3g`, which QuickTime, VLC, IINA and ffmpeg call mov_text), labelled with its language. The picture and
//! sound are never touched.

use super::{JoinError, Kind, Sample, Spool, Track};

/// One line of text on screen, from `start` to `end` seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// One language's subtitles. `offset`: what turns a cue's time into the stream's own clock (seconds), from HLS's
/// X-TIMESTAMP-MAP; `None` counts cues from the stream's start.
#[derive(Clone, Debug, PartialEq)]
pub struct Subtitles {
    /// The language as the stream gives it (`en`, `pt-BR`); empty when it gives none.
    pub language: String,
    pub cues: Vec<Cue>,
    pub offset: Option<f64>,
}

/// `00:01:02.500` or `01:02.500` → seconds.
fn clock(text: &str) -> Option<f64> {
    let (hms, frac) = text.trim().split_once('.').unwrap_or((text.trim(), "0"));
    let parts: Vec<f64> = hms.split(':').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let whole = match parts.as_slice() {
        [h, m, s] => h * 3600.0 + m * 60.0 + s,
        [m, s] => m * 60.0 + s,
        _ => return None,
    };
    let frac: f64 = format!("0.{frac}").parse().ok()?;
    Some(whole + frac)
}

/// The cue text without WebVTT's markup (`<i>`, `<c.yellow>`, `<v Name>`, timestamps) and with its entities read.
fn plain(text: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&lt;", "<").replace("&gt;", ">").replace("&nbsp;", "\u{a0}").replace("&lrm;", "").replace("&rlm;", "").replace("&amp;", "&")
}

/// Reads a WebVTT file: its cues, and the offset its X-TIMESTAMP-MAP gives (HLS: `MPEGTS` in 90 kHz ticks is the
/// stream time of `LOCAL`).
pub fn parse_vtt(text: &str) -> Result<(Vec<Cue>, Option<f64>), String> {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n").replace('\r', "\n");
    if !text.trim_start().starts_with("WEBVTT") {
        return Err("This isn't a WebVTT file".into());
    }
    let mut offset = None;
    let mut cues = Vec::new();
    for block in text.split("\n\n") {
        let lines: Vec<&str> = block.lines().filter(|l| !l.trim().is_empty()).collect();
        if let Some(map) = lines.iter().find_map(|l| l.strip_prefix("X-TIMESTAMP-MAP=")) {
            let (mut ticks, mut local) = (None, None);
            for part in map.split(',') {
                if let Some(v) = part.trim().strip_prefix("MPEGTS:") {
                    ticks = v.trim().parse::<f64>().ok();
                } else if let Some(v) = part.trim().strip_prefix("LOCAL:") {
                    local = clock(v);
                }
            }
            if let (Some(t), Some(l)) = (ticks, local) {
                offset = Some(t / 90_000.0 - l);
            }
        }
        let Some(at) = lines.iter().position(|l| l.contains("-->")) else { continue };
        if lines[0].starts_with("NOTE") || lines[0].starts_with("STYLE") || lines[0].starts_with("REGION") {
            continue;
        }
        let (from, rest) = lines[at].split_once("-->").unwrap_or_default();
        let to = rest.split_whitespace().next().unwrap_or("");
        let (Some(start), Some(end)) = (clock(from), clock(to)) else { continue };
        let body = lines[at + 1..].iter().map(|l| plain(l)).collect::<Vec<_>>().join("\n");
        if end > start && !body.trim().is_empty() {
            cues.push(Cue { start, end, text: body });
        }
    }
    Ok((cues, offset))
}

/// Which of `languages` (as a stream gives them) S15's `wanted` keeps, as indexes in `wanted`'s order: one per
/// language, `*` taking every one; `en` matches `en-US`.
pub fn pick(languages: &[String], wanted: &[String]) -> Vec<usize> {
    let primary = |code: &str| code.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
    let mut picked: Vec<usize> = if wanted.iter().any(|w| w == "*") {
        (0..languages.len()).collect()
    } else {
        wanted.iter().filter_map(|w| languages.iter().position(|l| primary(l) == primary(w))).collect()
    };
    let mut seen = Vec::new();
    picked.retain(|&i| {
        let l = primary(&languages[i]);
        let fresh = !seen.contains(&l);
        seen.push(l);
        fresh
    });
    picked
}

/// ISO 639-1 codes (and BCP 47 tags such as `pt-BR`) as the three letters an MP4 track's language takes (ISO 639-2/T);
/// `und` for none or one not known here.
pub fn three_letters(language: &str) -> String {
    let primary = language.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
    if primary.len() == 3 && primary.chars().all(|c| c.is_ascii_lowercase()) {
        return primary;
    }
    let known = [
        ("en", "eng"), ("de", "deu"), ("es", "spa"), ("fr", "fra"), ("it", "ita"), ("pt", "por"), ("nl", "nld"),
        ("sv", "swe"), ("da", "dan"), ("no", "nor"), ("nb", "nob"), ("nn", "nno"), ("fi", "fin"), ("pl", "pol"),
        ("cs", "ces"), ("sk", "slk"), ("el", "ell"), ("tr", "tur"), ("ru", "rus"), ("uk", "ukr"), ("ar", "ara"),
        ("he", "heb"), ("hi", "hin"), ("ja", "jpn"), ("ko", "kor"), ("zh", "zho"), ("id", "ind"), ("ms", "msa"),
        ("th", "tha"), ("vi", "vie"), ("hu", "hun"), ("ro", "ron"), ("bg", "bul"), ("hr", "hrv"), ("sr", "srp"),
        ("sl", "slv"), ("et", "est"), ("lv", "lav"), ("lt", "lit"), ("ca", "cat"), ("eu", "eus"), ("gl", "glg"),
        ("fa", "fas"), ("ur", "urd"), ("bn", "ben"), ("ta", "tam"), ("te", "tel"), ("fil", "fil"), ("tl", "tgl"),
    ];
    known.iter().find(|(two, _)| *two == primary).map(|(_, three)| three.to_string()).unwrap_or_else(|| "und".into())
}

/// The `tx3g` sample description: white text centred at the bottom, in the defaults ffmpeg writes (Arial, 16
/// points), so players read the text as plain, with no font changes.
fn entry() -> Vec<u8> {
    let mut e = Vec::new();
    e.extend_from_slice(&[0; 6]);
    e.extend_from_slice(&1u16.to_be_bytes()); // data reference
    e.extend_from_slice(&0u32.to_be_bytes()); // display flags
    e.push(1); // centred
    e.push(0xff); // at the bottom
    e.extend_from_slice(&[0, 0, 0, 0xff]); // background
    e.extend_from_slice(&[0; 8]); // text box: the whole picture
    // Style: characters 0–0, font 1, plain, 16 points, white.
    e.extend_from_slice(&[0, 0, 0, 0, 0, 1, 0, 16, 0xff, 0xff, 0xff, 0xff]);
    let font = b"Arial";
    let ftab_len = 8 + 2 + 2 + 1 + font.len();
    e.extend_from_slice(&(ftab_len as u32).to_be_bytes());
    e.extend_from_slice(b"ftab");
    e.extend_from_slice(&1u16.to_be_bytes());
    e.extend_from_slice(&1u16.to_be_bytes());
    e.push(font.len() as u8);
    e.extend_from_slice(font);
    let mut boxed = ((8 + e.len()) as u32).to_be_bytes().to_vec();
    boxed.extend_from_slice(b"tx3g");
    boxed.extend(e);
    boxed
}

/// A subtitle track from `from` seconds (the stream's start on its own clock) to `to`: each stretch of time shows
/// the cues on screen then (overlapping ones on separate lines), with empty samples between. `start` is where cue
/// time 0 is when the subtitles carry no X-TIMESTAMP-MAP. `None` when nothing falls inside.
pub fn track(subs: &Subtitles, start: f64, from: f64, to: f64, size: (u32, u32), spool: &mut Spool) -> Result<Option<Track>, JoinError> {
    const MS: f64 = 1000.0;
    let shift = subs.offset.unwrap_or(start);
    let cues: Vec<Cue> = subs
        .cues
        .iter()
        .map(|c| Cue { start: (c.start + shift).max(from), end: (c.end + shift).min(to), text: c.text.clone() })
        .filter(|c| c.end > c.start)
        .collect();
    if cues.is_empty() {
        return Ok(None);
    }
    // Every moment a cue starts or ends, in milliseconds on the stream's clock.
    let mut edges: Vec<i64> = vec![(from * MS).round() as i64];
    for c in &cues {
        edges.push((c.start * MS).round() as i64);
        edges.push((c.end * MS).round() as i64);
    }
    edges.sort_unstable();
    edges.dedup();
    let mut samples = Vec::new();
    for pair in edges.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let mid = (a + b) as f64 / 2.0 / MS;
        let text: Vec<&str> = cues.iter().filter(|c| c.start <= mid && mid < c.end).map(|c| c.text.as_str()).collect();
        let text = text.join("\n");
        let bytes = text.as_bytes();
        let len = bytes.len().min(u16::MAX as usize);
        let mut data = (len as u16).to_be_bytes().to_vec();
        data.extend_from_slice(&bytes[..len]);
        let offset = spool.put(&data)?;
        samples.push(Sample { dts: a, cts: 0, duration: (b - a) as u32, sync: true, offset, size: data.len() as u32 });
    }
    Ok(Some(Track {
        kind: Kind::Text,
        timescale: MS as u32,
        entry: entry(),
        width: size.0,
        height: size.1,
        language: three_letters(&subs.language),
        samples,
        edit: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webvtt_cues_and_their_timestamp_map() {
        let text = "WEBVTT\nX-TIMESTAMP-MAP=MPEGTS:133200,LOCAL:00:00:00.000\n\nNOTE a note\n\n1\n00:00:00.500 --> 00:00:01.500 align:middle\n<v Ann>Hello &amp; <i>welcome</i></v>\n\n00:01.800 --> 00:02.900\nTwo\nlines\n";
        let (cues, offset) = parse_vtt(text).unwrap();
        assert_eq!(offset, Some(1.48));
        assert_eq!(cues, vec![
            Cue { start: 0.5, end: 1.5, text: "Hello & welcome".into() },
            Cue { start: 1.8, end: 2.9, text: "Two\nlines".into() },
        ]);
        assert!(parse_vtt("not one").is_err());
    }

    #[test]
    fn languages_take_three_letters() {
        assert_eq!(three_letters("en"), "eng");
        assert_eq!(three_letters("pt-BR"), "por");
        assert_eq!(three_letters("deu"), "deu");
        assert_eq!(three_letters(""), "und");
        assert_eq!(three_letters("xx"), "und");
    }
}
