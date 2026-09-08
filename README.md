# Streaming GIF experiment

Can a browser’s `<img>` decode and animate a GIF **while the HTTP response is still open** and new frames keep arriving?

This repo is a lab for that question: a small Rust HTTP server that starts a valid GIF, then keeps sending frames over a chunked HTTP/1.1 body, with or without the formal trailer byte `0x3B`. The science lives in [`EXPERIMENT.md`](EXPERIMENT.md); run protocols and filled observation tables live in [`docs/lab/`](docs/lab/README.md).

## Why this is interesting

GIF is a **file format**, not a streaming protocol. Its blocks are sequential, though: after the header and colour table, each frame is a self-contained graphic-control + image block. In principle a decoder can paint the first frame and keep appending later ones before the trailer arrives.

The practical hypothesis is narrower:

> Incremental GIF *can* show a first frame before the file is complete, and later frames *can* update while the connection stays open — but real behaviour depends on the browser, HTTP buffering, and the image decoder.

The experiment treats that as something to measure, not assume.

## What the server does

Axum serves two routes on `127.0.0.1:3000`:

| Route | Role |
|-------|------|
| `/` | Page with `<img id="live-gif">`; `src` is assigned after `window.load` so an infinite stream does not block document completion |
| `/live.gif` | Chunked `image/gif` body, `Cache-Control: no-store`, no `Content-Length` |

Two modules own the GIF, not the HTTP handler:

- **`FrameGenerator`** — 64×64 synthetic scene: black background, 8×8 white block walking one pixel per frame.
- **`GifStream`** — GIF89a header, infinite-loop extension, one complete frame per HTTP chunk, then a **trailer policy**.

Trailer policy (env `GIF_TRAILER`):

| Value | Behaviour |
|-------|-----------|
| `after:N` (default `after:10`) | Finite GIF: N frames, then trailer `0x3B` |
| `never` | Open stream: frames forever, no trailer |
| `ondrop` | Best-effort trailer when the body is dropped (not reliable on the wire yet) |

Frame interval is `GIF_INTERVAL_MS` (default `1000`). The GIF graphic-control delay is derived from the same interval so send rate and playback delay stay aligned.

The `gif` crate always writes a trailer on `Drop`. Open-stream mode wraps the encoder in `ManuallyDrop` / `mem::forget` so that Drop never runs. “Flush” means **yield one HTTP chunk per frame**, not `Write::flush` on the encoder.

## Results (2026-09-08)

Ran on localhost, Ubuntu, HTTP/1.1, no reverse proxy: Chromium 152 (instrumented) and Firefox 155 (manual). Safari was unavailable.

**Overall: the structural hypothesis is confirmed on this stack.** Best observed scenario is early first paint plus ongoing updates while the connection stays open. Perceived “liveness” is dominated by frame interval, not by a long client-side buffer pause.

### Phase 1 — Finite GIF is valid

`AfterN(10)` produces a real GIF89a file: 64×64, 10 images, loop forever, delay 1.00 s, ends with `0x3B`. Chromium plays it both as a saved file and via `<img>` against the live finite response. That gate had to pass before any streaming claim.

### Phase 2 — Open stream paints live

With `GIF_TRAILER=never` at 1 s/frame, Chromium painted almost immediately after `src` was assigned and kept updating without reload (the white block walked and wrapped). Parallel `curl -N` grew over time, stayed chunked, and ended on `0x00` rather than `0x3B`. No long buffer pause. Classified **best case**.

A practical client detail: putting `/live.gif` in the HTML `src` keeps `document` from reaching `load` on an infinite response. The page assigns `src` after `window.load` for that reason.

### Phase 3 — Clients agree on this machine

| Client | Open stream at 1 s, no trailer |
|--------|--------------------------------|
| Chromium | Best — immediate first paint, continuous updates |
| Firefox 155 | Best — same live-stream behaviour (manual) |
| Safari | Not tested (unavailable on Linux) |
| `curl -N` | Bytes keep arriving; no visual decode |

Decoder variance across browsers was **not** the limiter here. Both visual clients behaved as a rudimentary live stream.

### Phase 4 — Interval sets the feel, not whether it works

All four rates updated while open. First paint was immediate at every rate.

| Interval | Perceived liveness |
|----------|--------------------|
| 100 ms | Closest to “live”: continuous motion |
| 500 ms | Still lively, denser than 1 s |
| 1 s | Predictable baseline (used in other phases) |
| 2 s | Discrete steps; least live |

The GIF delay + send interval is the practical limit of “live feel” on localhost. HTTP buffering was not.

### Phase 5 — How the stream ends matters

| Condition | Chromium |
|-----------|----------|
| **Abrupt close** (kill server, no `0x3B`) | `<img>` becomes the **broken-image icon**. Not a frozen last frame. `curl` file has no trailer. No auto-retry. |
| **Late trailer** (`after:20`) | After a clean `0x3B`, the decoder **loops the buffered animation**. Not freeze, not broken icon. |
| **Reconnect** | Ordinary reload starts a fresh stream immediately after either A or B. `Cache-Control: no-store` was enough; no cache-bust query needed. |

So: missing trailer is not “keep showing the last frame”; it is a decode failure. A late trailer turns the open stream into a normal looping GIF of whatever arrived.

### What actually limited liveness

- **Required:** explicit per-frame HTTP chunk yield (server/runtime).
- **Not observed on this path:** browser image-decoder hold, OS/network buffer pause, proxy buffering (there was no proxy).
- **Did limit the *feel*:** graphic-control delay and `GIF_INTERVAL_MS`.

## How to run

```bash
# Finite GIF (phase 1 default): 10 frames, then trailer
cargo run

# Open stream (phases 2–4)
GIF_TRAILER=never cargo run

# Late trailer (phase 5 B)
GIF_TRAILER=after:20 cargo run

# Faster / slower send + GIF delay
GIF_TRAILER=never GIF_INTERVAL_MS=100 cargo run
```

Open `http://127.0.0.1:3000/`. Inspect bytes without a decoder:

```bash
curl -N -v http://127.0.0.1:3000/live.gif -o /tmp/live.gif
# interrupt after a few seconds; header should be GIF89a
# Never policy: last byte is not 0x3B
xxd /tmp/live.gif | head
file /tmp/live.gif
```

Tests:

```bash
cargo test
```

Lab notes: start at [`docs/lab/README.md`](docs/lab/README.md). Synthesis after all five phases is in [`docs/lab/RESULTS.md`](docs/lab/RESULTS.md).

## Future experiments

The v1 run isolated encoder + browser on localhost. The next interesting unknowns are **middleboxes** and **termination engineering**, not “does GIF stream at all”.

**Worth doing next**

1. **Reverse proxy in front** (nginx, Caddy, Cloudflare). Buffering there is the likely next confound. Repeat phases 2 and 4 behind the proxy and see whether first paint delays or frames arrive in bursts.
2. **Reliable on-drop trailer.** `OnDrop` is best-effort today: client disconnect drops the Axum body, and a trailer may never leave the socket. If abrupt-close should become a looping GIF instead of a broken `<img>`, the trailer has to be flushed onto the wire on drop.
3. **Safari / WebKit.** Unavailable on the lab machine; it is the remaining major browser decoder.

**Later / optional**

- HTTP/2 vs HTTP/1.1 chunked. HTTP/1.1 already proved the hypothesis on localhost; HTTP/2 only matters if a proxy or browser treats the two differently.
- Manual binary GIF emission. The `gif` crate plus `ManuallyDrop` was enough for all five phases. Revisit only if `OnDrop` must deliver a trailer and the crate still fights that.
- Firefox (and Safari) repeat of phase 5. Chromium’s broken-icon vs loop-after-trailer split may not be universal.
- Unused independent variables from the original protocol: larger frames, busier scenes, local colour tables, no Netscape loop extension, remote host (not localhost).

**Probably not the next move**

Rewriting the encoder from scratch, or treating a failed open-stream in one new environment as a reason to throw away the server. Phases 2–5 failures in a *new* stack (proxy, HTTP/2, Safari) are scientific results; the encoder is already known-good from phase 1.
