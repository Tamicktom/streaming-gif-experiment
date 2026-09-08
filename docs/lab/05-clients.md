# 05 — Phase 3: Clients

Compare the same open stream across clients. Do not change timing in this phase.

## Objective

Run the phase 2 setup (1 s/frame, no trailer) on Chrome, Firefox, Safari if available, and `curl -N` as a non-browser control. One observation row per client.

## Prerequisites

- Phase 2 ([`04-open-stream.md`](04-open-stream.md)) completed (even if the scenario was “bad”)
- Same server config as phase 2

## Protocol

1. Keep trailer policy `Never`, interval **1 s**.
2. For each client below, hard-refresh or open a clean tab, load the page with `<img src="/live.gif">` (or run `curl` as specified).
3. Observe ~30–60 s per client.
4. Fill one table row per client. Do not retune interval mid-phase.

| Client | How to load |
|--------|-------------|
| Chrome | Static page + `<img>` |
| Firefox | Same page |
| Safari | Same page, if available; else mark N/A |
| `curl -N` | Bytes arriving over time (no visual decode); note growth / stall only |

## What not to change

- Frame interval (stay at 1 s)
- Trailer policy (`Never`)
- Scene, resolution, headers
- Adding a proxy “to see what happens” — that is a later confound, not this phase

## Completion checklist

- [x] Chrome row filled
- [x] Firefox row filled (or unavailable noted)
- [x] Safari row filled or N/A
- [x] `curl -N` row filled (byte growth yes/no)
- [x] Best-behaved **visual** client identified for phase 4

## Observation table

| Timestamp | Client | Interval | Trailer | First frame | Updates while open? | Scenario | Notes |
|-----------|--------|----------|---------|-------------|---------------------|----------|-------|
| 2026-09-08 ~19:29 UTC | Chromium (Cursor browser) — `<img>` on `http://127.0.0.1:3000/` | 1 s | Never | ~immediate after `src` assign | yes — white-block CSS centroid x 27.5 → 203.5 → 99.5 (wrap) over ~30 s, no reload | **best** | Server: `GIF_TRAILER=never`, `interval=1000ms`. Screenshots `phase3-chromium-t{0,15,30}.png`. Same deferred-`src` page as phase 2. No long buffer pause observed. |
| 2026-09-08 ~19:32 UTC | Firefox 155.0.1 — `<img>` on `http://127.0.0.1:3000/` | 1 s | Never | works normally (manual) | yes — updates while open, no reload | **best** | Manual observation by operator: same Never / 1 s setup; behaves like a normal live stream (matches Chromium). No MCP instrumentation. |
| 2026-09-08 | Safari | 1 s | Never | N/A | N/A | N/A | Unavailable on Linux (see `01-environment.md`). |
| 2026-09-08 ~19:29 UTC | curl -N | 1 s | Never | n/a | yes — bytes grew 143→353→878→1403→2033 B over ~18 s | n/a | `HTTP/1.1` chunked; `content-type: image/gif`; `cache-control: no-store`; no `Content-Length`; last byte `0x00` (not `0x3B`); gifsicle ~19 images at interrupt. Saved `/tmp/phase3.gif`. |

**Best visual client for phase 4:** Chromium (Cursor browser) — Firefox also **best**; keep Chromium for instrumented phase 4 runs.

## If it fails

| Symptom | Treat as | Action |
|---------|----------|--------|
| All browsers bad, `curl` grows | **Science** | Format may stream; decoders may not — continue timing/termination |
| One browser works, others do not | **Science** | Primary finding for this phase; use the working one in phase 4 |
| No client gets bytes | **Debug** | Server / network; do not proceed until `curl -N` shows growth |