//! HLS playlists (RFC 8216): a master playlist lists a stream's qualities and its separate sound tracks; a media
//! playlist lists one quality's pieces, with their keys and init section.

use reqwest::Url;

/// One quality of a stream.
#[derive(Clone, Debug, PartialEq)]
pub struct Variant {
    pub url: Url,
    /// Bits a second (BANDWIDTH).
    pub bandwidth: u64,
    /// Width and height (RESOLUTION), when given.
    pub resolution: Option<(u32, u32)>,
    pub codecs: Option<String>,
    /// The AUDIO group its sound comes from, when the sound is a separate track.
    pub audio: Option<String>,
    /// High dynamic range (VIDEO-RANGE=PQ or HLG): left out when the stream also has a standard version.
    pub hdr: bool,
    /// The SUBTITLES group its subtitles come from.
    pub subtitles: Option<String>,
}

/// A separate sound track (EXT-X-MEDIA TYPE=AUDIO).
#[derive(Clone, Debug, PartialEq)]
pub struct Rendition {
    pub group: String,
    pub name: String,
    pub language: Option<String>,
    pub default: bool,
    /// `None`: the sound is already in the variant's pieces.
    pub url: Option<Url>,
}

/// How a piece is encrypted (EXT-X-KEY).
#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    None,
    /// AES-128 in CBC mode with PKCS#7 padding, the key at `url`. Without an IV, the piece's sequence number is it.
    Aes128 { url: Url, iv: Option<[u8; 16]> },
    /// Anything else (SAMPLE-AES, FairPlay, Widevine…): protected, never downloaded.
    Protected(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub url: Url,
    /// Seconds (EXTINF).
    pub duration: f64,
    /// Part of the file at `url`: (length, offset) (EXT-X-BYTERANGE).
    pub range: Option<(u64, u64)>,
    /// The media sequence number.
    pub sequence: u64,
    pub key: Key,
    /// The init section the piece needs (EXT-X-MAP), for MP4 fragments: its address and byte range.
    pub map: Option<(Url, Option<(u64, u64)>)>,
    /// The timestamps may start again here (EXT-X-DISCONTINUITY).
    pub discontinuity: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Playlist {
    /// `subtitles`: EXT-X-MEDIA TYPE=SUBTITLES, each a playlist of WebVTT files.
    Master { variants: Vec<Variant>, audio: Vec<Rendition>, subtitles: Vec<Rendition> },
    /// `ended`: EXT-X-ENDLIST, or a VOD playlist; otherwise a live stream that keeps growing.
    Media { segments: Vec<Segment>, ended: bool },
}

/// `KEY=VALUE,KEY="VALUE, with commas"` → pairs.
fn attributes(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        let Some(eq) = rest.find('=') else { break };
        let key = rest[..eq].trim().to_string();
        rest = &rest[eq + 1..];
        let value = if let Some(quoted) = rest.strip_prefix('"') {
            let end = quoted.find('"').unwrap_or(quoted.len());
            let v = quoted[..end].to_string();
            rest = quoted.get(end + 1..).unwrap_or("");
            v
        } else {
            let end = rest.find(',').unwrap_or(rest.len());
            let v = rest[..end].trim().to_string();
            rest = &rest[end..];
            v
        };
        rest = rest.trim_start().strip_prefix(',').unwrap_or(rest).trim_start();
        out.push((key, value));
    }
    out
}

fn attr<'a>(pairs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    pairs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

/// "<length>[@<offset>]"; without an offset the range follows the previous one.
fn byterange(text: &str, previous_end: u64) -> Option<(u64, u64)> {
    let (len, off) = match text.split_once('@') {
        Some((l, o)) => (l.trim().parse().ok()?, o.trim().parse().ok()?),
        None => (text.trim().parse().ok()?, previous_end),
    };
    Some((len, off))
}

fn hex16(text: &str) -> Option<[u8; 16]> {
    let hex = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X"))?;
    let hex = format!("{hex:0>32}");
    if hex.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// Reads a playlist fetched from `base` (its address, which relative links are resolved against).
pub fn parse(text: &str, base: &Url) -> Result<Playlist, String> {
    let text = text.trim_start_matches('\u{feff}');
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    if lines.next() != Some("#EXTM3U") {
        return Err("This isn't an HLS playlist".into());
    }
    let join = |link: &str| base.join(link).map_err(|_| format!("A playlist link doesn't work: {link}"));
    let (mut variants, mut audio, mut segments) = (Vec::new(), Vec::new(), Vec::new());
    let mut subtitles = Vec::new();
    let mut pending_variant: Option<Vec<(String, String)>> = None;
    let (mut duration, mut range, mut discontinuity) = (0.0, None, false);
    let mut key = Key::None;
    let mut map = None;
    let (mut sequence, mut ended, mut range_end) = (0u64, false, 0u64);
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()).skip(1) {
        if let Some(rest) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            pending_variant = Some(attributes(rest));
        } else if let Some(rest) = line.strip_prefix("#EXT-X-MEDIA:") {
            let a = attributes(rest);
            let kind = attr(&a, "TYPE");
            if kind == Some("SUBTITLES") && attr(&a, "URI").is_some() {
                subtitles.push(Rendition {
                    group: attr(&a, "GROUP-ID").unwrap_or_default().to_string(),
                    name: attr(&a, "NAME").unwrap_or_default().to_string(),
                    language: attr(&a, "LANGUAGE").map(str::to_string),
                    // Forced subtitles (signs, foreign lines only) aren't the whole text: kept out of a choice.
                    default: attr(&a, "FORCED") == Some("YES"),
                    url: attr(&a, "URI").map(join).transpose()?,
                });
            }
            if kind == Some("AUDIO") {
                audio.push(Rendition {
                    group: attr(&a, "GROUP-ID").unwrap_or_default().to_string(),
                    name: attr(&a, "NAME").unwrap_or_default().to_string(),
                    language: attr(&a, "LANGUAGE").map(str::to_string),
                    default: attr(&a, "DEFAULT") == Some("YES"),
                    url: attr(&a, "URI").map(join).transpose()?,
                });
            }
        } else if let Some(rest) = line.strip_prefix("#EXTINF:") {
            duration = rest.split(',').next().unwrap_or("0").trim().parse().unwrap_or(0.0);
        } else if let Some(rest) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            range = byterange(rest, range_end);
        } else if let Some(rest) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            sequence = rest.trim().parse().unwrap_or(0);
        } else if line == "#EXT-X-DISCONTINUITY" {
            discontinuity = true;
        } else if line == "#EXT-X-ENDLIST" || line == "#EXT-X-PLAYLIST-TYPE:VOD" {
            ended = true;
        } else if let Some(rest) = line.strip_prefix("#EXT-X-KEY:") {
            let a = attributes(rest);
            key = match attr(&a, "METHOD").unwrap_or("NONE") {
                "NONE" => Key::None,
                "AES-128" => Key::Aes128 {
                    url: join(attr(&a, "URI").ok_or("An encrypted stream gives no key")?)?,
                    iv: attr(&a, "IV").and_then(hex16),
                },
                other => Key::Protected(other.to_string()),
            };
            // A key format other than the plain one is a DRM system's, whatever the method says.
            if attr(&a, "KEYFORMAT").is_some_and(|f| f != "identity") && key != Key::None {
                key = Key::Protected(attr(&a, "KEYFORMAT").unwrap_or_default().to_string());
            }
        } else if let Some(rest) = line.strip_prefix("#EXT-X-MAP:") {
            let a = attributes(rest);
            let url = join(attr(&a, "URI").ok_or("A stream's init section has no address")?)?;
            map = Some((url, attr(&a, "BYTERANGE").and_then(|r| byterange(r, 0))));
        } else if line.starts_with('#') {
            continue;
        } else if let Some(a) = pending_variant.take() {
            variants.push(Variant {
                url: join(line)?,
                bandwidth: attr(&a, "BANDWIDTH").and_then(|b| b.parse().ok()).unwrap_or(0),
                resolution: attr(&a, "RESOLUTION").and_then(|r| {
                    let (w, h) = r.split_once(['x', 'X'])?;
                    Some((w.parse().ok()?, h.parse().ok()?))
                }),
                codecs: attr(&a, "CODECS").map(str::to_string),
                audio: attr(&a, "AUDIO").map(str::to_string),
                hdr: matches!(attr(&a, "VIDEO-RANGE"), Some("PQ" | "HLG")),
                subtitles: attr(&a, "SUBTITLES").map(str::to_string),
            });
        } else {
            if let Some((len, off)) = range {
                range_end = off + len;
            }
            segments.push(Segment {
                url: join(line)?,
                duration,
                range: range.take(),
                sequence,
                key: key.clone(),
                map: map.clone(),
                discontinuity: std::mem::take(&mut discontinuity),
            });
            sequence += 1;
            duration = 0.0;
        }
    }
    if !variants.is_empty() {
        return Ok(Playlist::Master { variants, audio, subtitles });
    }
    if segments.is_empty() {
        return Err("The playlist lists nothing to download".into());
    }
    Ok(Playlist::Media { segments, ended })
}

impl Variant {
    /// Its picture is H.264 or H.265 (or not said): not VP8, VP9 or AV1, which some players can't open in an MP4.
    pub fn plays_everywhere(&self) -> bool {
        let codecs = self.codecs.as_deref().unwrap_or("").to_ascii_lowercase();
        !["vp08", "vp09", "vp8", "vp9", "av01"].iter().any(|c| codecs.contains(c))
    }
}

impl Playlist {
    /// The best quality: the most pixels, then H.264 or H.265 over VP9 or AV1, then the most bits a second.
    pub fn best_variant(&self) -> Option<&Variant> {
        self.pick_variant(None)
    }

    /// The quality to download (S13): the best one whose picture is at most `max` lines (its shorter side, so
    /// "1080p" means the same for a tall phone video), or the smallest when every one is bigger; the best with no
    /// `max`. HDR versions are left out when a standard one exists, and a stream that gives no sizes takes its best.
    /// Of two versions the same size, the one in H.264 or H.265 wins over VP9 or AV1 whatever their bit rates: every
    /// player opens it (QuickTime doesn't play VP9 in MP4, and YouTube's 1080p came as VP9 for its higher rate).
    pub fn pick_variant(&self, max: Option<u32>) -> Option<&Variant> {
        let Playlist::Master { variants, .. } = self else { return None };
        let standard: Vec<&Variant> = variants.iter().filter(|v| !v.hdr).collect();
        let pool = if standard.is_empty() { variants.iter().collect() } else { standard };
        let rank = |v: &&Variant| (v.resolution.map(|(w, h)| w as u64 * h as u64).unwrap_or(0), v.plays_everywhere(), v.bandwidth);
        let lines = |v: &Variant| v.resolution.map(|(w, h)| w.min(h));
        match max {
            Some(max) if pool.iter().any(|v| lines(v).is_some()) => pool
                .iter()
                .copied()
                .filter(|v| lines(v).is_some_and(|l| l <= max))
                .max_by_key(rank)
                .or_else(|| pool.iter().copied().filter(|v| lines(v).is_some()).min_by_key(rank)),
            _ => pool.into_iter().max_by_key(rank),
        }
    }

    /// The separate sound track a variant plays with: the default of its group, else its first.
    pub fn audio_for(&self, variant: &Variant) -> Option<&Rendition> {
        self.audio_in(variant, &[])
    }

    /// The subtitles a variant plays with that S15 keeps (`languages`: codes in order, or `*` for all), one per
    /// language, forced ones left out; `en` matches `en-US`.
    pub fn subtitles_in(&self, variant: &Variant, languages: &[String]) -> Vec<&Rendition> {
        let Playlist::Master { subtitles, .. } = self else { return Vec::new() };
        let primary = |code: &str| code.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
        let pool: Vec<&Rendition> = subtitles
            .iter()
            .filter(|r| !r.default && variant.subtitles.as_ref().is_none_or(|g| *g == r.group))
            .collect();
        let lang = |r: &Rendition| r.language.as_deref().map(primary).unwrap_or_default();
        let mut picked: Vec<&Rendition> = if languages.iter().any(|l| l == "*") {
            pool.clone()
        } else {
            languages.iter().filter_map(|want| pool.iter().copied().find(|r| lang(r) == primary(want))).collect()
        };
        let mut seen = Vec::new();
        picked.retain(|r| {
            let l = lang(r);
            let fresh = !seen.contains(&l);
            seen.push(l);
            fresh
        });
        picked
    }

    /// A variant that carries sound only (its codecs name no picture, and it has no size), the best of them: what a
    /// sound-only download takes when the sound isn't a separate track.
    pub fn sound_variant(&self) -> Option<&Variant> {
        let Playlist::Master { variants, .. } = self else { return None };
        let picture = ["avc", "hvc", "hev", "dvh", "dva", "vp0", "vp8", "vp9", "av01", "mp4v"];
        variants
            .iter()
            .filter(|v| v.resolution.is_none() && v.codecs.as_deref().is_some_and(|c| !picture.iter().any(|p| c.to_ascii_lowercase().contains(p))))
            .max_by_key(|v| v.bandwidth)
    }

    /// The separate sound track a variant plays with (S14): the first of `languages` its group has (`en` matches
    /// `en-US`), else the group's default (the stream's own sound), else its first.
    pub fn audio_in(&self, variant: &Variant, languages: &[String]) -> Option<&Rendition> {
        let Playlist::Master { audio, .. } = self else { return None };
        let group = variant.audio.as_deref()?;
        let in_group: Vec<&Rendition> = audio.iter().filter(|r| r.group == group && r.url.is_some()).collect();
        let primary = |code: &str| code.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
        languages
            .iter()
            .find_map(|want| in_group.iter().find(|r| r.language.as_deref().is_some_and(|l| primary(l) == primary(want))))
            .or_else(|| in_group.iter().find(|r| r.default))
            .or(in_group.first())
            .copied()
    }
}

/// The IV of a piece without one of its own: its sequence number, big-endian, in 16 bytes.
pub fn sequence_iv(sequence: u64) -> [u8; 16] {
    let mut iv = [0u8; 16];
    iv[8..].copy_from_slice(&sequence.to_be_bytes());
    iv
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://cdn.example.com/v/master.m3u8").unwrap()
    }

    #[test]
    fn a_master_playlist_picks_the_best_quality_and_its_sound() {
        let text = "#EXTM3U\n\
            #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"English\",DEFAULT=YES,LANGUAGE=\"en\",URI=\"a/en.m3u8\"\n\
            #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"Deutsch\",LANGUAGE=\"de\",URI=\"a/de.m3u8\"\n\
            #EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360,CODECS=\"avc1.4d401e,mp4a.40.2\",AUDIO=\"aud\"\n\
            360.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080,AUDIO=\"aud\"\n\
            https://other.example.com/1080.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=2500000,RESOLUTION=1280x720,AUDIO=\"aud\"\n\
            720.m3u8\n";
        let p = parse(text, &base()).unwrap();
        let best = p.best_variant().unwrap();
        assert_eq!(best.url.as_str(), "https://other.example.com/1080.m3u8");
        assert_eq!(best.resolution, Some((1920, 1080)));
        let sound = p.audio_for(best).unwrap();
        assert_eq!((sound.name.as_str(), sound.url.as_ref().unwrap().as_str()), ("English", "https://cdn.example.com/v/a/en.m3u8"));
        let Playlist::Master { variants, .. } = &p else { panic!() };
        assert_eq!(variants[0].codecs.as_deref(), Some("avc1.4d401e,mp4a.40.2"), "a quoted value keeps its comma");
    }

    #[test]
    fn a_media_playlist_lists_its_pieces_keys_and_ranges() {
        let text = "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXT-X-MEDIA-SEQUENCE:7\n\
            #EXT-X-MAP:URI=\"init.mp4\"\n\
            #EXT-X-KEY:METHOD=AES-128,URI=\"https://keys.example.com/k\",IV=0x1\n\
            #EXTINF:6.0,\nall.mp4\n#EXT-X-BYTERANGE:1000@200\n\
            #EXTINF:5.5,\n#EXT-X-BYTERANGE:500\nall.mp4\n\
            #EXT-X-DISCONTINUITY\n#EXT-X-KEY:METHOD=NONE\n#EXTINF:4,\nlast.m4s\n#EXT-X-ENDLIST\n";
        let Playlist::Media { segments, ended } = parse(text, &base()).unwrap() else { panic!() };
        assert!(ended);
        assert_eq!(segments.len(), 3);
        assert_eq!((segments[0].sequence, segments[0].range), (7, None));
        assert_eq!(segments[1].range, Some((500, 0)), "a range without an offset follows the previous one (none yet)");
        let mut iv = [0u8; 16];
        iv[15] = 1;
        assert_eq!(segments[0].key, Key::Aes128 { url: Url::parse("https://keys.example.com/k").unwrap(), iv: Some(iv) });
        assert_eq!(segments[2].key, Key::None);
        assert!(segments[2].discontinuity && !segments[1].discontinuity);
        assert_eq!(segments[2].map.as_ref().unwrap().0.as_str(), "https://cdn.example.com/v/init.mp4");
        assert_eq!(sequence_iv(7)[15], 7);
    }

    #[test]
    fn protected_streams_and_live_ones_are_told_apart() {
        let drm = "#EXTM3U\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://x\",KEYFORMAT=\"com.apple.streamingkeydelivery\"\n#EXTINF:4,\na.ts\n";
        let Playlist::Media { segments, ended } = parse(drm, &base()).unwrap() else { panic!() };
        assert!(matches!(segments[0].key, Key::Protected(_)));
        assert!(!ended, "no ENDLIST: live");
        assert!(parse("<html>", &base()).is_err());
    }

    fn variant(line: &str) -> String {
        format!("#EXT-X-STREAM-INF:{line},AUDIO=\"aud\"\n{}.m3u8\n", line.len())
    }

    #[test]
    fn the_chosen_quality_is_the_best_that_fits() {
        let text = format!(
            "#EXTM3U\n{}{}{}{}{}",
            variant("BANDWIDTH=800000,RESOLUTION=640x360"),
            variant("BANDWIDTH=2500000,RESOLUTION=1280x720"),
            variant("BANDWIDTH=3000000,RESOLUTION=1280x720"),
            variant("BANDWIDTH=5000000,RESOLUTION=1920x1080"),
            variant("BANDWIDTH=9000000,RESOLUTION=3840x2160,VIDEO-RANGE=PQ"),
        );
        let p = parse(&text, &base()).unwrap();
        let size = |max| p.pick_variant(max).map(|v| (v.resolution.unwrap().1, v.bandwidth));
        assert_eq!(size(None), Some((1080, 5_000_000)), "Highest leaves out HDR when a standard version exists");
        assert_eq!(size(Some(1080)), Some((1080, 5_000_000)));
        assert_eq!(size(Some(720)), Some((720, 3_000_000)), "of two 720p versions, the one with more bits a second");
        assert_eq!(size(Some(1000)), Some((720, 3_000_000)), "at most the lines asked for");
        assert_eq!(size(Some(240)), Some((360, 800_000)), "nothing that small: the smallest");

        // The same size in VP9 at a higher rate and in H.264: H.264, which every player opens (YouTube's 1080p).
        let youtube = format!(
            "#EXTM3U\n{}{}{}",
            variant("BANDWIDTH=5710503,RESOLUTION=1920x1080,CODECS=\"vp09.00.40.08,mp4a.40.2\""),
            variant("BANDWIDTH=4342644,RESOLUTION=1920x1080,CODECS=\"avc1.640028,mp4a.40.2\""),
            variant("BANDWIDTH=1494335,RESOLUTION=1280x720,CODECS=\"vp09.00.31.08,mp4a.40.2\""),
        );
        let y = parse(&youtube, &base()).unwrap();
        assert_eq!(y.pick_variant(None).unwrap().bandwidth, 4_342_644, "H.264 over VP9 at the same size");
        assert_eq!(y.pick_variant(Some(720)).unwrap().bandwidth, 1_494_335, "VP9 when it's the only one that size");

        let hdr_only = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=9000000,RESOLUTION=3840x2160,VIDEO-RANGE=PQ\nuhd.m3u8\n";
        assert!(parse(hdr_only, &base()).unwrap().pick_variant(Some(1080)).is_some(), "only HDR: it is taken");

        let tall = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1,RESOLUTION=720x1280\na.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=2,RESOLUTION=1080x1920\nb.m3u8\n";
        let pick = parse(tall, &base()).unwrap().pick_variant(Some(720)).unwrap().url.path().to_string();
        assert_eq!(pick, "/v/a.m3u8", "a tall video's 720p is 720 wide");

        let sizeless = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\na.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=2\nb.m3u8\n";
        assert_eq!(parse(sizeless, &base()).unwrap().pick_variant(Some(480)).unwrap().bandwidth, 2, "no sizes: the best");
    }

    #[test]
    fn the_sound_track_follows_the_preferred_languages() {
        let text = "#EXTM3U\n\
            #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"Original\",DEFAULT=YES,LANGUAGE=\"ja\",URI=\"ja.m3u8\"\n\
            #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"English\",LANGUAGE=\"en-US\",URI=\"en.m3u8\"\n\
            #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"Deutsch\",LANGUAGE=\"de\",URI=\"de.m3u8\"\n\
            #EXT-X-STREAM-INF:BANDWIDTH=1,RESOLUTION=1280x720,AUDIO=\"aud\"\nv.m3u8\n";
        let p = parse(text, &base()).unwrap();
        let v = p.best_variant().unwrap();
        let name = |langs: &[&str]| p.audio_in(v, &langs.iter().map(|l| l.to_string()).collect::<Vec<_>>()).unwrap().name.clone();
        assert_eq!(name(&[]), "Original", "no preference: the stream's own sound");
        assert_eq!(name(&["en"]), "English", "en matches en-US");
        assert_eq!(name(&["fr", "DE"]), "Deutsch", "the first preferred language the stream has");
        assert_eq!(name(&["fr"]), "Original", "none of them: the stream's own");
    }

    #[test]
    fn a_sound_only_variant_is_found_by_its_codecs() {
        let base = Url::parse("https://example.com/m.m3u8").unwrap();
        let text = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=900000,RESOLUTION=640x360,CODECS=\"avc1.4d401e,mp4a.40.2\"\nv.m3u8\n\
                    #EXT-X-STREAM-INF:BANDWIDTH=64000,CODECS=\"mp4a.40.5\"\nlow.m3u8\n\
                    #EXT-X-STREAM-INF:BANDWIDTH=128000,CODECS=\"mp4a.40.2\"\nhigh.m3u8\n";
        let p = parse(text, &base).unwrap();
        assert_eq!(p.sound_variant().unwrap().url.as_str(), "https://example.com/high.m3u8", "the best sound-only variant");
        let none = parse("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=900000,RESOLUTION=640x360\nv.m3u8\n", &base).unwrap();
        assert!(none.sound_variant().is_none(), "without codecs, a variant may hold a picture");
    }
}
