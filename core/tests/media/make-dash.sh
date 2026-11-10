#!/bin/sh
# Makes the DASH test streams in this folder with ffmpeg (committed, so the tests run anywhere):
#   dash-ondemand  one file per quality with its index (sidx), as Facebook serves reels: SegmentBase with
#                  Initialization and indexRange byte ranges, video and sound in separate files
#   dash-template  numbered pieces with a timeline (SegmentTemplate + SegmentTimeline), as ffmpeg writes DASH
#   dash-number    numbered pieces of a fixed length (SegmentTemplate with $Number$ and duration)
set -eu
cd "$(dirname "$0")"
Q="-hide_banner -loglevel error -y"
V="-f lavfi -i testsrc2=size=160x90:rate=25"
A="-f lavfi -i sine=frequency=440:sample_rate=44100"
X264="-c:v libx264 -profile:v high -bf 2 -g 25 -b:v 60k -x264-params log-level=none"
FRAG="-movflags +frag_keyframe+empty_moov+default_base_moof+global_sidx -frag_duration 2000000"
rm -rf dash-ondemand dash-template dash-number
mkdir dash-ondemand dash-template dash-number
ffmpeg $Q $V -t 6 $X264 -an $FRAG dash-ondemand/video.mp4
ffmpeg $Q $A -t 6 -c:a aac -b:a 32k $FRAG dash-ondemand/audio.mp4
python3 - <<'PY'
import struct
def ranges(path):
    d = open(path, 'rb').read()
    i, boxes = 0, {}
    while i < len(d):
        size, kind = struct.unpack('>I4s', d[i:i + 8])
        boxes.setdefault(kind.decode(), (i, i + size - 1))
        i += size
    return boxes['moov'][1], boxes['sidx']
v_init, (v_s, v_e) = ranges('dash-ondemand/video.mp4')
a_init, (a_s, a_e) = ranges('dash-ondemand/audio.mp4')
open('dash-ondemand/manifest.mpd', 'w').write(f'''<?xml version="1.0" encoding="UTF-8"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" profiles="urn:mpeg:dash:profile:isoff-on-demand:2011" type="static" mediaPresentationDuration="PT6S" minBufferTime="PT2S">
<Period id="0" duration="PT6S">
<AdaptationSet id="0" contentType="video" subsegmentAlignment="true">
<Representation id="low" bandwidth="40000" codecs="avc1.64000b" mimeType="video/mp4" width="80" height="45"><BaseURL>missing-low.mp4</BaseURL><SegmentBase indexRange="0-1"><Initialization range="0-1"/></SegmentBase></Representation>
<Representation id="v" bandwidth="60000" codecs="avc1.64000b" mimeType="video/mp4" width="160" height="90"><BaseURL>video.mp4?_nc_cat=1&amp;oh=x</BaseURL><SegmentBase indexRange="{v_s}-{v_e}" timescale="12800"><Initialization range="0-{v_init}"/></SegmentBase></Representation>
</AdaptationSet>
<AdaptationSet id="1" contentType="audio">
<Representation id="a" bandwidth="32000" codecs="mp4a.40.2" mimeType="audio/mp4" audioSamplingRate="44100"><BaseURL>audio.mp4?_nc_cat=2&amp;oh=y</BaseURL><SegmentBase indexRange="{a_s}-{a_e}" timescale="44100"><Initialization range="0-{a_init}"/></SegmentBase></Representation>
</AdaptationSet>
</Period>
</MPD>
''')
PY
ffmpeg $Q $V $A -t 6 -map 0:v -map 1:a $X264 -c:a aac -b:a 32k -f dash -seg_duration 2 -use_template 1 -use_timeline 1 \
    -adaptation_sets "id=0,streams=v id=1,streams=a" dash-template/manifest.mpd
ffmpeg $Q $V $A -t 6 -map 0:v -map 1:a $X264 -c:a aac -b:a 32k -f dash -seg_duration 2 -use_template 1 -use_timeline 0 \
    -adaptation_sets "id=0,streams=v id=1,streams=a" dash-number/manifest.mpd
