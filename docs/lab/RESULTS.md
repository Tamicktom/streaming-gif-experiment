# Results (fill after phases 1–5)

Synthesis only. Do not fill until [`07-termination.md`](07-termination.md) is done. Point back to phase tables instead of re-copying every row.

## Run metadata

| Field | Value |
|-------|-------|
| Date | 2026-09-08 |
| Machine / OS | `tamicktom-ryzen` / Ubuntu 26.04 LTS (kernel 7.0.0-31-generic, x86_64) |
| Rust / Cargo version | rustc / cargo 1.96.1 |
| Server commit / description | Axum `127.0.0.1:3000`, `gif` crate Encoder + `GifStream` trailer policies (`Never` / `AfterN` / `OnDrop`); env `GIF_TRAILER`, `GIF_INTERVAL_MS`; 64×64 synthetic white block |
| Best visual client (phase 3) | Chromium (Cursor browser); Firefox also **best** (manual) |
| Preferred interval (phase 4) | 1 s for predictable baseline; 100 ms closest to “live” feel |

## Hypothesis

See [`EXPERIMENT.md`](../../EXPERIMENT.md).

| Statement | Outcome |
|-----------|---------|
| Incremental GIF can show a first frame before the file is complete | **confirmed** |
| Subsequent frames update while the connection stays open | **confirmed** |
| Behaviour depends on browser / HTTP buffering / decoder | **partially confirmed** — Chromium and Firefox both “best” here; Safari N/A; termination paths differ sharply (abrupt close vs late trailer) |

**Overall:** confirmed (on this localhost Chromium/Firefox stack)

## Best observed scenario

Pick one and cite the phase note:

- Best case (early paint + ongoing updates)
- Intermediate (buffer then bursts)
- Bad (wait for end / single frame / no live update)

**Which:** Best case (early paint + ongoing updates)  
**Where recorded:** phase 2 ([`04-open-stream.md`](04-open-stream.md)); same pattern in phases 3–4

## Buffering and the HTTP stack

What actually limited “liveness”?

- [x] Server / runtime (needed explicit flush)
- [ ] Browser image decoder
- [ ] Browser or OS network buffer
- [x] Something else: GIF graphic-control delay + chosen `GIF_INTERVAL_MS` (rate of motion, not whether updates happen)

Short note:

On localhost with chunk-per-frame flush and no proxy, Chromium painted immediately and tracked send rate from 100 ms through 2 s ([`06-timing.md`](06-timing.md)). No long client-side buffer pause was observed. The main practical limit of “live feel” was interval (2 s steppy; 100 ms continuous), not HTTP buffering. Explicit per-frame yield of body chunks was required by design (see [`02-architecture.md`](02-architecture.md)).

## Termination behaviour (phase 5)

| Condition | One-line summary |
|-----------|------------------|
| Abrupt close without trailer | `<img>` becomes broken-image icon; curl ends without `0x3B` ([`07-termination.md`](07-termination.md) A) |
| Late trailer | Clean `0x3B` after ~20 frames; browser loops buffered GIF ([`07-termination.md`](07-termination.md) B) |
| Reconnect | Ordinary reload starts a fresh stream immediately after A or B; no cache-bust needed ([`07-termination.md`](07-termination.md) C) |

## Follow-ups

| Question | Decision |
|----------|----------|
| Pursue manual binary GIF emission? | **later** — `gif` crate + `ManuallyDrop` for `Never` was enough for all phases; revisit only if `OnDrop` must deliver a trailer on the wire |
| Worth testing behind a reverse proxy? | **yes** — buffering there is the likely next confound |
| Worth HTTP/2 comparison? | **later** — HTTP/1.1 chunked already proved the hypothesis on localhost |
| Other | Optional: Firefox repeat of phase 5; make `OnDrop` flush trailer to the socket if that policy is needed |

## One-paragraph conclusion

On this machine, an open HTTP/1.1 chunked GIF (`Never` trailer, no proxy) is a workable visual pseudo-stream in Chromium and Firefox: first paint is early and frames keep updating while the connection stays open, with perceived liveness dominated by frame interval rather than browser buffering. Closing without a trailer breaks the `<img>` (error icon); sending a late trailer completes the file and the decoder loops the buffered animation; reconnecting with a normal reload fetches a fresh stream under `Cache-Control: no-store`. The experiment’s structural hypothesis is confirmed for this stack; the next interesting unknowns are reverse-proxy buffering and whether a reliable on-drop trailer is worth engineering.
