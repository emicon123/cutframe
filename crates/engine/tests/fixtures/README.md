# Engine test fixtures

Small, licence-clean media made with the pinned LGPL FFmpeg build (`scripts/ffmpeg.json`,
n7.1.5-12-g1fdbca85aa) and only FFmpeg's built-in sources and encoders. Kept in the repo so tests
never depend on files outside it.

| File | Content |
|---|---|
| `testsrc-64x64-1s.mp4` (7.4 KB) | 1 s, 64x64, 25 fps `testsrc2` pattern as MPEG-4 Part 2 (`mpeg4`) + 1 s 440 Hz mono 48 kHz sine as AAC-LC (native `aac`) |

Command (any OS, `ffmpeg` from the pinned build):

```bash
ffmpeg -f lavfi -i "testsrc2=size=64x64:rate=25:duration=1" \
  -f lavfi -i "sine=frequency=440:sample_rate=48000:duration=1" \
  -map 0:v -map 1:a -c:v mpeg4 -g 25 -q:v 20 -pix_fmt yuv420p -c:a aac -b:a 16k -ac 1 \
  -map_metadata -1 -fflags +bitexact -flags:v +bitexact -flags:a +bitexact -movflags +faststart \
  testsrc-64x64-1s.mp4
```
