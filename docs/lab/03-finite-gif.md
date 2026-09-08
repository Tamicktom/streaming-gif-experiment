# 03 — Phase 1: Finite GIF (structural validation)

Prove the endpoint can emit a **valid closed GIF** before testing open streams.

## Objective

Generate a fixed number of frames (e.g. 10), send the trailer `0x3B`, and confirm normal animation as a finished file.

## Prerequisites

- [`01-environment.md`](01-environment.md) tools ready
- [`02-architecture.md`](02-architecture.md) decisions accepted
- Server implements `/live.gif` with trailer policy `AfterN(10)` (or equivalent)

## Protocol

1. Start the server on localhost (no proxy).
2. Configure finite mode: **10 frames**, then trailer.
3. Save the response to disk:

   ```bash
   curl -N http://127.0.0.1:3000/live.gif -o /tmp/phase1.gif
   ```

4. Inspect:

   ```bash
   file /tmp/phase1.gif
   xxd /tmp/phase1.gif | head
   # optional:
   gifsicle --info /tmp/phase1.gif
   ```

5. Open `/tmp/phase1.gif` in a browser (file URL or drag-and-drop) **and** open the page that uses `<img src="/live.gif">` while the finite endpoint still serves a complete response.
6. Confirm the animation plays through the frames and loops (or ends) as a normal GIF.

## What not to change

- Resolution / palette / scene (keep the v1 synthetic frame)
- Frame interval (any fixed interval is fine; do not tune for “streaming feel” yet)
- HTTP version / proxies
- Trailer must be present for this phase

## Completion checklist

- [x] Saved file is recognized as GIF (`file` / magic `GIF89a`)
- [x] Trailer byte present (ends with `0x3B`) or encoder confirms closed file
- [x] Frame count matches expectation (~10) if tooling reports it
- [x] Browser plays the animation from the saved file
- [x] Same animation looks correct via `<img>` against a finite response

**Gate:** if any of the above fail, **stop**. Fix encoding/streaming before phase 2. An invalid finite GIF cannot support incremental-stream conclusions.

## Observation table

| Timestamp | Client | Interval | Trailer | What you saw |
|-----------|--------|----------|---------|--------------|
| 2026-09-08 ~16:06 UTC | curl -N → `/tmp/phase1.gif` | 1 s | AfterN(10) | `file`: GIF89a 64×64; ends `0x3B`; `gifsicle --info`: 10 images, loop forever, delay 1.00s; chunked, `image/gif`, `no-store`, no `Content-Length` |
| 2026-09-08 | Chromium (Cursor browser) — saved file via `http://127.0.0.1:8765/phase1.gif` | 1 s | AfterN(10) | Animation plays; white 8×8 block moves right across frames (Pillow: 10 frames, x centroids 3.5…12.5) |
| 2026-09-08 | Chromium (Cursor browser) — `<img src="/live.gif">` on `http://127.0.0.1:3000/` | 1 s | AfterN(10) | Live finite response animates; screenshot white-centroid shifted left→right (31.5→43.5) ~2s apart; `/live.gif` HTTP 200 |

## If it fails

| Symptom | Treat as | Action |
|---------|----------|--------|
| Not a GIF / decode error | **Debug** | Header, palette, LZW, incomplete frames |
| Static single frame | **Debug** | Graphic control / delay / loop extension |
| Animates only after download completes in finite mode | Unlikely for a closed file; re-check you really got a finished response | Re-run `curl` save and open file offline |
| Animates offline but not via HTTP | **Debug** | Headers (`Content-Type`), caching |