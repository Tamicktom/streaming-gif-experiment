# 01 — Environment

Prepare a clean localhost path so buffering outside the app is not in the way.

## Goal

Have Rust tooling, browsers, and byte-inspection tools ready so you can serve `/live.gif` on `127.0.0.1` with no reverse proxy.

## Prerequisites

None. Do this before implementing or running any phase.

## What you need

| Tool | Why |
|------|-----|
| Rust + Cargo | Build and run the server |
| Chrome | Primary browser observation |
| Firefox | Second browser for phase 3 |
| Safari | Optional, if available |
| `curl` | Stream bytes without a decoder (`-N` disables buffering) |
| `xxd` or `hexdump` | Inspect GIF header / trailer bytes |
| `file` | Quick type check on a saved GIF |
| `gifsicle` | Optional; `gifsicle --info` for frame counts |

## Rules for this experiment

1. **Localhost only** — prefer `http://127.0.0.1:<port>`, not a remote host.
2. **No reverse proxy** — no nginx, Caddy, Cloudflare, etc. in front of the app for v1 runs.
3. **HTTP/1.1** — keep the stack simple; do not force HTTP/2 for the first passes.
4. **Direct connection** — browser → Axum. Any middlebox is a confound.

## How to inspect the response

Useful commands once the server exists (phases 1+):

```bash
# Non-buffered download; watch bytes arrive over time
curl -N -v http://127.0.0.1:3000/live.gif -o /tmp/live.gif

# First bytes should look like a GIF header (GIF89a)
xxd /tmp/live.gif | head

# Confirm type
file /tmp/live.gif
```

What to look for in `curl -v`:

- `Content-Type: image/gif`
- Transfer looks **chunked** (or streaming) — typically **no** `Content-Length` for an open stream
- Connection stays open while frames are generated

## Completion checklist

Mark when true for your machine (server may still be stub until you implement):

- [x] `rustc` / `cargo` available (`cargo --version`) — rustc/cargo 1.96.1 (rustup stable)
- [x] Chrome available — Chromium 152.0.7977.64 (snap); Google Chrome not installed
- [x] Firefox available (or noted as unavailable) — Mozilla Firefox 155.0.1
- [x] Safari noted as available / unavailable — unavailable (Linux)
- [x] `curl` available — curl 8.18.0
- [x] `xxd` or `hexdump` available — both present
- [x] You can open `http://127.0.0.1:<port>/` once the app listens (after implementation) — `http://127.0.0.1:3000/`
- [x] You can hit `/live.gif` with `curl -N` once the endpoint exists — phase 1 save to `/tmp/phase1.gif`

## Notes

_Date / machine / OS:_ 2026-09-08 / `tamicktom-ryzen` / Ubuntu 26.04 LTS (kernel 7.0.0-31-generic, x86_64)

_Anything non-standard (VPN, corporate proxy, unusual Rust toolchain):_
- Primary browser is Chromium snap, not Google Chrome; Safari N/A on Linux.
- `gifsicle` 1.95 built from source into `~/.local` (apt/`sudo` not available in this session); `file` 5.46 present.
- rustc/cargo 1.96.1 via rustup stable; no `HTTP_PROXY`/`HTTPS_PROXY` env vars.
- Ports 8000, 8001, and 8080 already listening — use **3000** for the experiment.
- HTTP/1.1 direct to Axum; no reverse proxy in front for v1 runs.
