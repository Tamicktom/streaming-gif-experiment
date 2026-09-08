# 08 — Video source (MP4 → incremental GIF)

Follow-up lab: does incremental GIF still live-update when frames come from a real MP4 instead of the synthetic walking block?

This is **not** a redo of phases 1–5. Those already confirmed open-stream GIF on localhost. Here the independent variable is the **frame source**.

## Objective

Decode `static/example.mp4` (or `GIF_VIDEO_PATH`) frame by frame with ffmpeg, encode each RGB frame as a GIF image block, and stream chunks on `/video.gif`. Observe whether `<img>` paints early and keeps updating.

## Prerequisites

- Phases 1–2 complete (finite + open synthetic stream known-good)
- `ffmpeg` and `ffprobe` on `PATH`
- Example file present: `static/example.mp4` (gitignored; ~742 MB AV1 2560×1080 @ 60 fps)

## Downsample defaults

Full-resolution 60 fps GIF is not viable. Lab defaults (confirmed on this machine):

| Knob | Value | Why |
|------|-------|-----|
| Scale | `scale=480:-2` → 480×202 | Fits `<img>`; NeuQuant stays affordable |
| FPS | `fps=10` | Matches GIF delay 10 cs (100 ms) |
| Interval | `GIF_INTERVAL_MS` default **100** for `/video.gif` | Align send pace with decode fps |
| Trailer | default **ondrop** (play once → trailer on EOF) | Override with `GIF_TRAILER=never` for open stream |
| Palette | per-frame local via `Frame::from_rgb` | Video needs more than 2 colours |

Synthetic control remains at `/live.gif`.

## Protocol

1. Place (or keep) the sample at `static/example.mp4`, or set `GIF_VIDEO_PATH`.
2. Open stream (recommended first observation):

   ```bash
   GIF_TRAILER=never cargo run
   ```

   Play-once (default trailer policy for video):

   ```bash
   cargo run
   ```

3. Open `http://127.0.0.1:3000/` — page assigns `/video.gif` after `window.load`.
4. Watch for 30–60 s:
   - When does the first frame appear?
   - Do later frames update without reload?
   - Any long pause (decode / NeuQuant / buffering)?
5. Parallel byte check:

   ```bash
   curl -N -v http://127.0.0.1:3000/video.gif -o /tmp/video.gif
   # interrupt after a few seconds
   xxd /tmp/video.gif | head
   file /tmp/video.gif
   ```

6. Optional control: open `/live.gif` directly and confirm the synthetic stream still behaves as in phase 2.

## What not to change

- No reverse proxy
- Localhost HTTP/1.1 only
- Keep scale/fps defaults for the first run (change only after a baseline observation)

## Completion checklist

- [ ] Server starts and logs video path / trailer / interval
- [ ] `/video.gif` returns `image/gif`, `Cache-Control: no-store`, no `Content-Length`
- [ ] Body starts with `GIF89a`
- [ ] Browser first paint observed
- [ ] Ongoing updates observed (or failure classified)
- [ ] `curl -N` shows growing bytes

## Observation table

| Field | Value |
|-------|-------|
| Date | |
| Client | |
| `GIF_TRAILER` | |
| `GIF_INTERVAL_MS` | |
| First paint | |
| Ongoing updates? | |
| Classification (best / intermediate / bad) | |
| Notes (CPU load, stalls, broken icon, …) | |

## If it fails

| Symptom | Likely cause | Action |
|---------|--------------|--------|
| 404 on `/video.gif` | Missing MP4 | Add `static/example.mp4` or set `GIF_VIDEO_PATH` |
| 500 / ffmpeg spawn error | ffmpeg not installed | Install ffmpeg; re-run |
| Bytes grow, no paint | Decoder / size / delay | Compare with `/live.gif`; try `GIF_INTERVAL_MS=200` |
| First frame only then stall | Encode too slow for interval | Lower scale or fps in code; note as scientific result |
| Broken `<img>` mid-stream | Abrupt close without trailer | Expected under kill; use `never` + leave connection open |
