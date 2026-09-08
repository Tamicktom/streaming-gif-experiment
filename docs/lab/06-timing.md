# 06 — Phase 4: Timing sensitivity

Vary frame interval on the best visual client from phase 3. Watch latency and whether the decoder “holds” or skips frames.

## Objective

Run the open stream at **100 ms**, **500 ms**, **1 s**, and **2 s** per frame. Record perceived lag and continuity for each rate.

## Prerequisites

- Phase 3 ([`05-clients.md`](05-clients.md)) done
- Best visual client named in that note
- Server can change interval without changing trailer policy

## Protocol

1. Fix client = best from phase 3; trailer = `Never`; no proxy.
2. For each interval in order: **100 ms → 500 ms → 1 s → 2 s**:
   - Restart or reconfigure the server so the new interval is clear.
   - Hard-refresh the page.
   - Observe ~30–60 s (longer at 2 s if needed to see several frames).
   - Note: time to first paint, smoothness, whether frames bunch up, whether the image seems stuck then jumps.
3. Fill one table row per interval.

## What not to change

- Client (one browser only)
- Trailer policy (`Never`)
- Scene / resolution / palette
- Do not introduce Safari/Firefox mid-phase unless phase 3 left you with no working client

## Completion checklist

- [x] 100 ms row filled
- [x] 500 ms row filled
- [x] 1 s row filled
- [x] 2 s row filled
- [x] Short note: which rate felt closest to “live”

## Observation table

| Timestamp | Client | Interval | Trailer | First frame | Visual latency | Continuity / jumps | Notes |
|-----------|--------|----------|---------|-------------|----------------|--------------------|-------|
| 2026-09-08 ~19:37 UTC | Chromium (Cursor browser) — `<img>` on `http://127.0.0.1:3000/` | 100 ms | Never | immediate | tracks send rate — continuous motion | smooth with frequent wraps (~6 s); one wide-span sample (t2) | Server: `GIF_TRAILER=never` `GIF_INTERVAL_MS=100` (`interval=100ms`). Centroids cx 235.5→71.5→…→107.5→179.5→91.5→195.5→243.5 over ~25 s. Screenshots `phase4-100ms-t{0,05,1,2,4,6,8,10,15,20,25}.png`. curl ~1 KB/s growth. |
| 2026-09-08 ~19:41 UTC | Chromium (Cursor browser) — `<img>` on `http://127.0.0.1:3000/` | 500 ms | Never | immediate | still lively; motion denser than 1 s | smooth with wraps; large Δcx between samples (catch-up / sample lag), no freeze | Server: `interval=500ms`. Centroids cx 111.5→187.5→31.5 (wrap)→…→67.5 over ~30 s. Screenshots `phase4-500ms-t{0..4,6,8,10,15,20,25,30}.png`. curl ~2 frames/s growth. |
| 2026-09-08 ~19:46 UTC | Chromium (Cursor browser) — `<img>` on `http://127.0.0.1:3000/` | 1 s | Never | immediate | matches phase 2/3 baseline | smooth; wrap mid-run; one wide-span sample (t15) | Server: `interval=1000ms`. Centroids cx 63.5→139.5→195.5→44.6 (wrap)→91.5→143.5→219.5 over ~30 s. Screenshots `phase4-1s-t{0,5,10,15,20,25,30}.png`. curl ~105 B/s. |
| 2026-09-08 ~19:49 UTC | Chromium (Cursor browser) — `<img>` on `http://127.0.0.1:3000/` | 2 s | Never | immediate | discrete steps; least “live” | hold-then-advance; clearest stepwise motion; wrap ~t40 | Server: `interval=2000ms`. Centroids cx 51.5→83.5→111.5→143.5→187.5→231.5→23.0 (wrap)→75.5 over ~50 s. Screenshots `phase4-2s-t{0,5,10,15,20,30,40,50}.png`. curl grew every ~2 s. |

**Interval closest to real-time feel:** 100 ms — continuous motion and frequent wraps; all four rates updated while open (no rate failed). 1 s remains the most predictable baseline; 2 s feels steppy / lagged.

## If it fails

| Symptom | Treat as | Action |
|---------|----------|--------|
| Fast intervals worse (more buffering) | **Science** | Note; common with client buffers |
| Only slow intervals update | **Science** | Decoder or browser may batch |
| No interval updates | Re-check phase 2/3; if still broken | **Debug** flush/interval config |