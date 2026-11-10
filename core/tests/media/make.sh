#!/bin/sh
# Makes the small HLS test streams in this folder with ffmpeg (Homebrew's, on the Mac). They're committed, so the
# tests run anywhere; ffmpeg is only needed again to change them. Six seconds each, 160x90, in 2-second pieces.
set -eu
cd "$(dirname "$0")"
SRC="-f lavfi -i testsrc2=size=160x90:rate=25 -f lavfi -i sine=frequency=440:sample_rate=44100 -t 6"
V264="-c:v libx264 -profile:v high -bf 2 -g 25 -b:v 60k -x264-params log-level=none"
AAC="-c:a aac -b:a 32k"
Q="-hide_banner -loglevel error -y"
HLS="-f hls -hls_time 2 -hls_playlist_type vod"
rm -rf h264-ts h264-fmp4 hevc-ts mp3-ts split aes
mkdir h264-ts h264-fmp4 hevc-ts mp3-ts split aes
ffmpeg $Q $SRC $V264 $AAC $HLS -hls_segment_filename h264-ts/s%d.ts h264-ts/index.m3u8
ffmpeg $Q $SRC $V264 $AAC $HLS -hls_segment_type fmp4 -hls_fmp4_init_filename init.mp4 \
    -hls_segment_filename h264-fmp4/s%d.m4s h264-fmp4/index.m3u8
ffmpeg $Q $SRC -c:v libx265 -tag:v hvc1 -b:v 60k -x265-params log-level=none:bframes=3 -g 25 $AAC $HLS \
    -hls_segment_filename hevc-ts/s%d.ts hevc-ts/index.m3u8
ffmpeg $Q $SRC $V264 -c:a libmp3lame -b:a 32k $HLS -hls_segment_filename mp3-ts/s%d.ts mp3-ts/index.m3u8
ffmpeg $Q $SRC -map 0:v -map 1:a $V264 $AAC $HLS -master_pl_name master.m3u8 \
    -var_stream_map "v:0,agroup:aud a:0,agroup:aud,default:yes,language:en" \
    -hls_segment_filename "split/%v_s%d.ts" "split/%v.m3u8"
# Packed audio (HLS §3.4, as YouTube sends its sound): split's sound pieces as bare AAC (ADTS) frames, each behind an
# ID3 tag whose PRIV frame com.apple.streaming.transportStreamTimestamp holds its first frame's 90 kHz time; the
# picture is split's own.
rm -rf packed && mkdir packed
cp split/0.m3u8 split/0_s*.ts packed/
sed 's/1\.m3u8/a.m3u8/' split/master.m3u8 > packed/master.m3u8
sed 's/1_s\([0-9]\)\.ts/a\1.aac/' split/1.m3u8 > packed/a.m3u8
for ts in split/1_s*.ts; do
    n="${ts##*_s}"; n="${n%.ts}"
    pts="$(ffprobe -v error -select_streams a -show_entries packet=pts -of csv=p=0 "$ts" | head -1 | cut -d, -f1)"
    python3 -c 'import struct,sys
o=b"com.apple.streaming.transportStreamTimestamp\0"+struct.pack(">Q",int(sys.argv[1]))
f=b"PRIV"+bytes([0,0,0,len(o)])+b"\0\0"+o
sys.stdout.buffer.write(b"ID3\4\0\0"+bytes([0,0,0,len(f)])+f)' "$pts" > "packed/a$n.aac"
    ffmpeg $Q -i "$ts" -c:a copy -f adts - >> "packed/a$n.aac"
done
printf '0123456789abcdef' > aes/key.bin
printf 'key.bin\n%s/aes/key.bin\n00000000000000000000000000000001\n' "$PWD" > aes.keyinfo
ffmpeg $Q $SRC $V264 $AAC $HLS -hls_key_info_file aes.keyinfo -hls_segment_filename aes/s%d.ts aes/index.m3u8
rm aes.keyinfo
