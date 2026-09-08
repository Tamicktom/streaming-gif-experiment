# 07 — Phase 5: Termination tolerance

Probe how clients behave when the stream ends without a formal trailer, when a trailer arrives late, and when the client reconnects.

## Objective

One observation per condition:

1. **Abrupt close** — stop writing / drop connection **without** trailer `0x3B`
2. **Late trailer** — stream for a while (`Never`-like), then send trailer and close cleanly
3. **Reconnect** — after end or failure, reload / new `<img>` request; note whether a fresh stream starts cleanly

## Prerequisites

- Phase 4 ([`06-timing.md`](06-timing.md)) completed (or skipped only if you already know interval effects and document why)
- Ability to choose trailer policy and to kill the connection
- Prefer the best visual client and a comfortable interval from phase 4 (e.g. 1 s)

## Protocol

### Condition A — Abrupt close without trailer

1. Start open stream (`Never`).
2. Confirm a few frames visible (or bytes flowing).
3. Stop the server or abort the response without writing `0x3B`.
4. Note: last frame frozen? error icon? retry? console noise?

### Condition B — Late trailer

1. Stream for ~15–30 s without trailer.
2. Send trailer and end the body cleanly.
3. Note: animation stops, loops from buffered frames, or odd decoder state?

### Condition C — Reconnect

1. After A or B, reload the page or force a new request to `/live.gif`.
2. Note: new stream works immediately, stuck on old image, need hard refresh / cache bust?

## What not to change

Within each condition, keep client and interval fixed.
Do not change scene mid-condition.
Still no reverse proxy for these baseline runs.

## Completion checklist

- [x] Condition A observed and recorded
- [x] Condition B observed and recorded
- [x] Condition C observed and recorded
- [x] Ready to fill [`RESULTS.md`](RESULTS.md)

## Observation table

| Timestamp | Client | Interval | Condition | Trailer | What you saw |
|-----------|--------|----------|-----------|---------|--------------|
| 2026-09-08 ~21:01 UTC | Chromium (Cursor browser) — `<img>` on `http://127.0.0.1:3000/` | 1 s | A abrupt close | none | Before kill: live updates (cx 59.5→159.5→35.5 wrap over ~10 s). After `fuser -k` on server: `<img>` becomes **broken-image icon** (tiny mountain placeholder; white blob shrinks to ~84 px, green/gray icon colors). Not a frozen last frame. No GIF-related console errors (only CursorBrowser dialog override warning). No auto-retry. Parallel `curl -N` → `/tmp/phase5-a.gif`: size 12211 B, last byte `0x00` (not `0x3B`), gifsicle ~116 images. Server: `GIF_TRAILER=never` `GIF_INTERVAL_MS=1000`. Screenshots `phase5-a-t{0,5,10}.png`, `phase5-a-after-kill{,-t3}.png`. |
| 2026-09-08 ~21:04–21:06 UTC | Chromium (Cursor browser) — `<img>` on `http://127.0.0.1:3000/` | 1 s | B late trailer | late `0x3B` (`after:20`) | Streamed ~20 s Never-like then clean close. While open: cx 87.5→27.5 (wrap)→67.5. After trailer: animation **continues looping buffered frames** (cx 43.5→95.5→91.5→67.5→47.5 over ~20 s post-trailer); not freeze, not broken icon, no odd decoder state. Console clean. `curl -N` to EOF: `/tmp/phase5-b.gif` 2139 B, ends `0x3B`, gifsicle 20 images, loop forever, delay 1.00s. Screenshots `phase5-b-t{0,10,20}.png`, `phase5-b-after-trailer-t{0,4,9,14,20}.png`. |
| 2026-09-08 ~21:03 / ~21:07 UTC | Chromium (Cursor browser) — `<img>` on `http://127.0.0.1:3000/` | 1 s | C reconnect | n/a | **After A:** server restarted `never`; ordinary reload → fresh stream immediately (cx 151.5→239.5 in ~5 s); no stale image, no cache-bust needed. **After B:** reload with `after:20` still up → fresh stream immediately (cx 71.5→51.5 wrap); again no stale/`?t=`. `Cache-Control: no-store` sufficient. Screenshots `phase5-c-after-a-t{0,5}.png`, `phase5-c-after-b-t{0,5}.png`. |

## If it fails

| Symptom | Treat as | Action |
|---------|----------|--------|
| Crash / tab hang on abrupt close | **Science** (+ note severity) | Record; optional follow-up with another browser |
| Late trailer ignored / loop broken | **Science** | Expected variance across decoders |
| Reconnect shows stale image | **Debug** or caching | Check `Cache-Control: no-store`; hard refresh |