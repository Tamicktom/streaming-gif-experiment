# 04 — Phase 2: Open stream (no trailer)

Test whether `<img>` updates while the connection stays open and frames keep arriving.

## Objective

Emit header + frames at **1 frame per second**, never send trailer (`Never` policy), keep the connection open, and observe progressive display.

Success / failure criteria: see [`EXPERIMENT.md`](../../EXPERIMENT.md) (“Critério de sucesso”). Summarize after the run: best / intermediate / bad case.

## Prerequisites

- Phase 1 ([`03-finite-gif.md`](03-finite-gif.md)) completion checklist marked — valid finite GIF proven
- Endpoint can run with trailer policy `Never`
- Page with `<img src="/live.gif" />` served from the same origin

## Protocol

1. Start the server; ensure open-stream mode (no trailer).
2. Open the static page in one browser (start with Chrome unless you have a reason not to).
3. Load `/live.gif` via the `<img>` (hard-refresh if needed; `Cache-Control: no-store` should help).
4. Watch for at least 30–60 seconds:
   - When does the first frame appear?
   - Do later frames appear without reloading?
   - Any long pause that looks like buffering?
5. In parallel, optionally confirm bytes keep arriving:

   ```bash
   curl -N -v http://127.0.0.1:3000/live.gif -o /tmp/phase2.gif
   # interrupt after ~15s; file should grow without a clean trailer requirement
   ```

6. Classify the run (see Expected scenarios in `EXPERIMENT.md`):
   - **Best:** early first paint + ongoing updates
   - **Intermediate:** delayed start, then updates in bursts
   - **Bad:** waits for response end / only first frame / bytes flow but no animation update

## What not to change

- Interval fixed at **1 s/frame** for this phase
- One primary browser only (comparisons are phase 3)
- No proxy, HTTP/1.1
- Scene / resolution unchanged from phase 1
- Do not send trailer during the observation window

## Completion checklist

- [x] Connection stayed open while frames were generated
- [x] First-frame timing noted (approximate is fine)
- [x] Whether subsequent frames updated without reload noted
- [x] Scenario classified: best / intermediate / bad
- [x] Observation row filled below

Phase 2 “failure” (bad case) is a **result**, not a blocker for phase 3 — still compare clients.

## Observation table

| Timestamp | Client | Interval | Trailer | First frame | Updates while open? | Scenario | Notes |
|-----------|--------|----------|---------|-------------|---------------------|----------|-------|
| 2026-09-08 ~16:34 UTC | Chromium (Cursor browser) — `<img>` on `http://127.0.0.1:3000/` | 1 s | Never | ~immediate after `src` assign (~1 s) | yes — white-block CSS centroid x 67.5 → 183.5 → 87.5 (wrap) over ~25 s, no reload | **best** | Server: `GIF_TRAILER=never`. Sync `<img src="/live.gif">` keeps `document` from reaching `load` (infinite tab spinner) — blocked MCP navigate; page now assigns `src` on `window.load`. `curl -N`: file grew 353→878→1403 B over ~13 s; chunked; `no-store`; no `Content-Length`; ends `0x00` not `0x3B`; gifsicle ~13–15 images. No long buffer pause observed. |

## If it fails

| Symptom | Treat as | Action |
|---------|----------|--------|
| Bytes never leave server / `curl -N` idle | **Debug** | Flush, body stream, task hung |
| `curl` grows but `<img>` blank forever | **Science** (or browser buffer) | Note bad/intermediate; continue to phase 3 |
| Only first frame, then freeze | **Science** | Record; try other clients in phase 3 |
| Works in `curl` save after interrupt but never live | **Science** + light **debug** | Confirm no `Content-Length`, no proxy |