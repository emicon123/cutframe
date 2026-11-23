#!/usr/bin/env bash
# Regenerates the media fixtures in this folder (see README.md). Needs `ffmpeg` from the pinned
# LGPL build (scripts/ffmpeg.json) on PATH and Python 3. Uses only FFmpeg's built-in test sources
# and LGPL-compatible encoders, so every output is licence-clean.
#
# Usage: bash generate.sh   (from any directory; writes next to this script)
set -euo pipefail
cd "$(dirname "$0")"

ff() { ffmpeg -hide_banner -loglevel error -y "$@"; }
# Deterministic output: no encoder/muxer version strings, no creation time.
EXACT=(-map_metadata -1 -fflags +bitexact -flags:v +bitexact -flags:a +bitexact)
SRC_V="testsrc2=size=64x64:rate=30"
tools() { python3 fixture_tools.py "$@"; }
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# --- Video --------------------------------------------------------------------------------------
# CFR H.264 + stereo 48 kHz AAC, 1 s.
ff -f lavfi -i "$SRC_V:duration=1" -f lavfi -i "sine=frequency=440:sample_rate=48000:duration=1" \
  -c:v libopenh264 -b:v 100k -pix_fmt yuv420p -c:a aac -b:a 32k -ac 2 "${EXACT[@]}" \
  -movflags +faststart h264-aac.mp4

# Portrait phone clip: HEVC, display matrix "rotate 90 degrees clockwise", VFR (frames 10-19 of
# 60 at 30 fps dropped, so one frame lasts 11/30 s). Encoded, then remuxed with the rotation.
ff -f lavfi -i "$SRC_V:duration=2" -vf "select='not(between(n\,10\,19))'" -fps_mode passthrough \
  -c:v libkvazaar -kvazaar-params preset=ultrafast -pix_fmt yuv420p "${EXACT[@]}" "$tmp/vfr.mov"
ff -display_rotation:v:0 -90 -i "$tmp/vfr.mov" -c copy "${EXACT[@]}" hevc-portrait-cw90-vfr.mov

# Anamorphic MPEG-2: 144x108 stored, SAR 4:3, so 192x108 on screen.
ff -f lavfi -i "testsrc2=size=144x108:rate=25:duration=0.4" -vf setsar=4/3 -c:v mpeg2video \
  -q:v 10 "${EXACT[@]}" mpeg2-anamorphic.mpg

# Browser-style WebM (VP9 + Opus) written to a pipe, so the container has no duration.
ff -f lavfi -i "$SRC_V:duration=1" -f lavfi -i "sine=frequency=330:sample_rate=48000:duration=1" \
  -c:v libvpx-vp9 -deadline realtime -cpu-used 8 -b:v 50k -c:a libopus -b:a 24k "${EXACT[@]}" \
  -f webm pipe:1 > vp9-opus-no-duration.webm

ff -f lavfi -i "$SRC_V:duration=0.5" -c:v libvpx -deadline realtime -cpu-used 8 -b:v 50k \
  "${EXACT[@]}" vp8.webm
ff -f lavfi -i "$SRC_V:duration=0.5" -c:v libaom-av1 -cpu-used 8 -crf 50 -pix_fmt yuv420p \
  "${EXACT[@]}" av1.mkv
ff -f lavfi -i "$SRC_V:duration=0.3" -c:v prores_ks -profile:v 0 "${EXACT[@]}" prores.mov
ff -f lavfi -i "$SRC_V:duration=0.3" -c:v mjpeg -q:v 10 -pix_fmt yuvj420p "${EXACT[@]}" mjpeg.avi
ff -f lavfi -i "$SRC_V:duration=0.5" -c:v mpeg4 -q:v 15 "${EXACT[@]}" -f mp4 mpeg4.m4v

# OBS-style recording: two audio tracks, the second one marked default.
ff -f lavfi -i "$SRC_V:duration=0.5" -f lavfi -i "sine=frequency=440:sample_rate=48000:duration=0.5" \
  -f lavfi -i "sine=frequency=880:sample_rate=44100:duration=0.5" -map 0:v -map 1:a -map 2:a \
  -c:v libopenh264 -b:v 50k -c:a aac -b:a 24k -ac:a:0 2 -ac:a:1 1 \
  -disposition:a:0 0 -disposition:a:1 default "${EXACT[@]}" two-audio-tracks.mkv

# HDR metadata. The LGPL build has no 10-bit HEVC encoder (kvazaar is 8-bit), so the HLG clip is
# 8-bit HEVC tagged BT.2020/HLG with an iPhone-style Dolby Vision 8.4 record (dvvC), and the PQ
# clip is 10-bit AV1 tagged BT.2020/PQ with mastering display (1000 nits) and MaxCLL (400) boxes.
ff -f lavfi -i "$SRC_V:duration=0.3" \
  -vf "setparams=color_primaries=bt2020:color_trc=arib-std-b67:colorspace=bt2020nc:range=tv" \
  -c:v libkvazaar -kvazaar-params preset=ultrafast -tag:v hvc1 \
  "${EXACT[@]}" "$tmp/hlg.mov"
tools dolby-vision "$tmp/hlg.mov" hevc-hlg-dv84.mov hvc1 8 4 4
ff -f lavfi -i "$SRC_V:duration=0.3" \
  -vf "format=yuv420p10le,setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc:range=tv" \
  -c:v libaom-av1 -cpu-used 8 -crf 50 "${EXACT[@]}" "$tmp/pq.mp4"
tools hdr10-boxes "$tmp/pq.mp4" av1-10bit-pq.mp4 av01 1000 400 120

# --- Audio --------------------------------------------------------------------------------------
A="sine=frequency=440:duration=0.5"
ff -f lavfi -i "$A:sample_rate=44100" -ac 2 -c:a libmp3lame -b:a 32k "${EXACT[@]}" stereo-44k.mp3
ff -f lavfi -i "$A:sample_rate=48000" -c:a pcm_s16le "${EXACT[@]}" mono-48k.wav
ff -f lavfi -i "$A:sample_rate=96000" -ac 2 -c:a flac "${EXACT[@]}" stereo-96k.flac
ff -f lavfi -i "$A:sample_rate=48000" -ac 2 -c:a libopus -b:a 24k "${EXACT[@]}" opus.ogg
ff -f lavfi -i "$A:sample_rate=44100" -c:a aac -b:a 24k "${EXACT[@]}" aac.m4a

# --- Images -------------------------------------------------------------------------------------
I="testsrc2=size=64x48:rate=1"
ff -f lavfi -i "$I" -frames:v 1 -c:v mjpeg -q:v 5 -pix_fmt yuvj420p "${EXACT[@]}" "$tmp/plain.jpg"
tools exif-orientation "$tmp/plain.jpg" exif-orientation-6.jpg 6
ff -f lavfi -i "$I" -frames:v 1 "${EXACT[@]}" image.png
# Straight alpha: a horizontal ramp from transparent (left) to opaque (right).
ff -f lavfi -i "$I" -frames:v 1 -vf "format=rgba,geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='255*X/63'" \
  -pix_fmt rgba "${EXACT[@]}" image-alpha.png
ff -f lavfi -i "$I" -frames:v 1 -c:v libwebp -quality 50 "${EXACT[@]}" image.webp
ff -f lavfi -i "$I" -frames:v 1 -pix_fmt bgr24 "${EXACT[@]}" image.bmp
ff -f lavfi -i "$I" -frames:v 1 -pix_fmt rgb24 -compression_algo deflate "${EXACT[@]}" image.tiff
ff -f lavfi -i "testsrc2=size=32x32:rate=5:duration=0.6" -c:v gif "${EXACT[@]}" animated.gif

# Phone-style HEIC: a 2x1 grid of 64x64 HEVC tiles (128x64 picture) rotated 90 degrees
# counter-clockwise by `irot` (irot turns counter-clockwise; 90 ccw = 270 cw).
for i in 0 1; do
  ff -f lavfi -i "testsrc2=size=64x64:rate=1" -vf "hue=h=$((i * 120))" -frames:v 1 \
    -c:v libkvazaar -kvazaar-params preset=ultrafast -pix_fmt yuv420p "${EXACT[@]}" \
    -f hevc "$tmp/tile$i.hevc"
done
tools heif-grid image-grid.heic 64 64 2 1 90 "$tmp/tile0.hevc" "$tmp/tile1.hevc"

# --- Broken files -------------------------------------------------------------------------------
tools truncate h264-aac.mp4 truncated.mp4 40
# A recording that crashed: Matroska written live (to a pipe: no duration, no cues), cut off in
# the middle of its last cluster. About 1.2 s of the 2 s survive.
ff -f lavfi -i "$SRC_V:duration=2" -f lavfi -i "sine=frequency=440:sample_rate=48000:duration=2" \
  -c:v libopenh264 -b:v 50k -g 15 -c:a aac -b:a 24k -cluster_time_limit 500 "${EXACT[@]}" \
  -f matroska pipe:1 > "$tmp/live.mkv"
tools truncate "$tmp/live.mkv" crashed-recording.mkv 60
tools random random-bytes.mp4 4096 20261122
printf 'This is a text file, not a QuickTime movie.\n' > text.mov
: > empty.mp4

# --- Non-ASCII path -----------------------------------------------------------------------------
mkdir -p "ąęłó"
ff -f lavfi -i "testsrc2=size=32x32:rate=10:duration=0.3" -c:v mpeg4 -q:v 20 "${EXACT[@]}" \
  "ąęłó/zażółć.mp4"

du -ch -- * "ąęłó"/* | tail -1
