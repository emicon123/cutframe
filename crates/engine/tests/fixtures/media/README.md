# Media format fixtures

Tiny, licence-clean files for the probe and format-coverage tests (`crates/engine/tests/media_formats.rs`,
design `docs/design/media-import.md` §10). About 320 KB in total (budget: 2 MB).

Every file is made by `generate.sh` from FFmpeg's built-in test sources (`testsrc2`, `sine`) with
the pinned LGPL build (`scripts/ffmpeg.json`, n7.1.5) and only LGPL-compatible encoders
(`libopenh264`, `libkvazaar`, `libvpx` (VP8 and VP9), `libaom-av1`, `libopus`, `libmp3lame`, `libwebp` and
FFmpeg's native ones). No camera footage, no third-party content. The output is bit-exact, so
running the script again gives the same bytes.

```bash
# from the repo root, with the pinned ffmpeg on PATH
bash crates/engine/tests/fixtures/media/generate.sh
```

`fixture_tools.py` (Python 3, standard library only) adds what the FFmpeg command line cannot
write: EXIF orientation, a Dolby Vision configuration box, HDR10 mastering/light-level boxes, a
HEIF tile-grid container, and the broken files.

| File | Made from | Checks |
|---|---|---|
| `h264-aac.mp4` | 1 s 64x64 30 fps H.264 (`libopenh264`) + stereo 48 kHz AAC, faststart | duration, codec names, channels, SDR has no HDR info |
| `numbered-gop.mp4` | 2 s 64x32 30 fps MPEG-4 Part 2 (`mpeg4`, 2 B-frames, keyframes at frames 0, 12, 21, 30, 39, 48, 57); frame n shows n in binary as eight 8-pixel bars (`geq`) | which source frame a render shows: clip speed, reverse across GOP boundaries, freeze frames (`tests/golden.rs`, reverse reader tests) |
| `hevc-portrait-cw90-vfr.mov` | 2 s HEVC (`libkvazaar`), frames 10–19 dropped (VFR), display matrix −90° (`-display_rotation`) | `rotation = cw90`, frame times from timestamps, sample-and-hold at 20 times |
| `mpeg2-anamorphic.mpg` | 144x108 MPEG-2, SAR 4:3 | `display_size` 192x108 |
| `vp9-opus-no-duration.webm` | VP9 + Opus written to a pipe (no duration in the container) | duration from the last packet |
| `vp8.webm`, `av1.mkv`, `prores.mov`, `mjpeg.avi`, `mpeg4.m4v` | VP8 (`libvpx`), AV1 (`libaom-av1`, decoded by dav1d), ProRes (`prores_ks`), MJPEG, MPEG-4 Part 2 in an MP4 named `.m4v` | probe + first-frame decode |
| `two-audio-tracks.mkv` | H.264 + stereo 48 kHz AAC + mono 44.1 kHz AAC, the second track marked default | both tracks listed, default first |
| `hevc-hlg-dv84.mov` | 8-bit HEVC tagged BT.2020 / HLG (`setparams`) + a `dvvC` box: profile 8, level 4, compatibility id 4 (iPhone style) | `hdr.transfer = hlg`, Dolby Vision fields, colour names |
| `av1-10bit-pq.mp4` | 10-bit AV1 tagged BT.2020 / PQ + `mdcv` (1000 nits) and `clli` (MaxCLL 400, MaxFALL 120) boxes | `bitDepth = 10`, `hdr.transfer = pq`, nits |
| `stereo-44k.mp3`, `mono-48k.wav`, `stereo-96k.flac`, `opus.ogg`, `aac.m4a` | 0.5 s sine | `kind = audio`, sample rate, channels |
| `sine-1k-48k.flac` | 1 s full-scale 1 kHz stereo sine, 48 kHz, 16-bit FLAC (`aevalsrc`) | audio reader decodes exactly, sample-accurate seeks, split points without a seek, pre-seeked jump cuts, volume 0.5 peaks at 0.5 (`tests/audio_mix.rs`); the ignored device test |
| `ramp-48k.flac` | 2 s stereo ramp from -0.8 to 0.8, 48 kHz, 16-bit FLAC (`aevalsrc`) | reversed voices play the samples backwards exactly, across a 1 s window; a reversed 2x voice descends (`tests/audio_pace.rs`) |
| `bursts-48k.flac` | 6 s stereo silence with a 40 ms Hann-shaped 1 kHz burst centred on every multiple of 0.5 s from 0.5 s, 48 kHz FLAC | the time stage puts content where the clip mapping says (sync at 0.5x..4x, reverse, seeks, split points, jump cuts, 48 vs 96 kHz parity) |
| `sine-1k-44k-mono.flac` | 1 s half-scale 1 kHz mono sine, 44.1 kHz, FLAC | resampling to 48 kHz in phase, mono on both channels at full level |
| `vfr-audio-offset.ts` | MPEG-TS (starts at 1.4 s): 1 s 32x32 MPEG-4 video with frames 10-19 dropped (VFR) + mono 48 kHz MP2 sine starting about 0.25 s later (`-itsoffset`) | `origin`, silence before late audio, sample-accurate seeks with decoder preroll |
| `exif-orientation-6.jpg` | 64x48 JPEG + APP1 Exif with Orientation = 6 | `rotation = cw90`, display 48x64 |
| `image.png`, `image.webp`, `image.bmp`, `image.tiff` | 64x48 still | `kind = image`, no duration |
| `image-alpha.png` | 64x48 RGBA PNG, alpha ramp from 0 (left column) to 255 (right column) | image layers keep straight alpha (`tests/readers.rs`) |
| `animated.gif` | 3 frames at 5 fps | `kind = video` |
| `image-grid.heic` | HEIF: a 2x1 `grid` of 64x64 HEVC tiles (128x64) with `irot` = 90° counter-clockwise | `kind = image`, grid size, `rotation = cw270` |
| `truncated.mp4` | the first 40 % of `h264-aac.mp4` | `damaged` |
| `crashed-recording.mkv` | 2 s H.264 + AAC Matroska written to a pipe (no duration, no cues, 0.5 s clusters), cut to its first 60 % mid-cluster | imports as video, duration ≈ 1.1 s from the last good packet (FFmpeg 7.1 stops cleanly at the cut; tail read errors are unit-tested) |
| `random-bytes.mp4` | 4096 pseudo-random bytes (seed 20261122) | `unsupportedFormat` |
| `text.mov` | a line of text | `unsupportedFormat` |
| `empty.mp4` | 0 bytes | `unsupportedFormat` |
| `ąęłó/zażółć.mp4` | 32x32 MPEG-4 Part 2 under a non-ASCII folder and file name | opens (UTF-8 paths; UTF-16 on Windows) |

Notes:

- The LGPL build has no 10-bit HEVC encoder (`libkvazaar` is 8-bit only), so the HLG clip is
  8-bit HEVC and the 10-bit check uses AV1. The HDR probe reads container metadata only, so the
  codec does not matter for it.
- `libopenh264` made the H.264 test files on a developer machine. The app uses it only as the
  last-resort export encoder (`engine::export::encoders`, export.md decision F28).
- FFmpeg 7.1 exposes the HEIF grid as a stream group but does not read `irot`; the engine reads it
  itself (`crates/engine/src/media/heif.rs`).
