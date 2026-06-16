# Explore: nanofish vs picoserve

Status: **exploration only — not decided, not started.** Captures whether to swap
the `kiln-app` HTTP server (`picoserve` 0.18) for [`nanofish`](https://crates.io/crates/nanofish).

## What nanofish is

- no_std HTTP **client + server** on top of `embassy-net`. ([github](https://github.com/rttfd/nanofish))
- **no-alloc**: zero-copy responses borrowed from a user-provided buffer; you own
  the memory, buffer sizes are compile-time const generics.
- Server is **HTTP-only** (no TLS). Client does TLS via `embedded-tls`.
- Server API = implement one trait, `match` on `request.path` yourself:

  ```rust
  impl HttpHandler for MyHandler {
      async fn handle_request(&mut self, req: &HttpRequest<'_>)
          -> Result<HttpResponse<'_>, nanofish::Error> {
          match req.path {
              "/"           => Ok(HttpResponse { status_code: StatusCode::Ok, headers: Vec::new(), body: ResponseBody::Text("...") }),
              "/api/status" => Ok(/* ... */),
              _             => Ok(/* 404 */),
          }
      }
  }
  ```

- v0.12.0 on crates.io. `defmt` / `log` features (mutually exclusive).

## Why it looked attractive

- **no-alloc + caller-owned buffers** → predictable RAM. picoserve leans on `alloc`
  and each worker future is **~84 KB** (the single biggest static-RAM lever — see
  `kiln-app/src/api.rs:16` and `server.rs:60`). That's the real itch.
- Smaller, embassy-native, no TAIT/ITIAT router machinery.
- **Flat handler ≈ flat stack frame.** nanofish's API is one `match` on `req.path`,
  not a deeply-nested generic `Router` type. That sidesteps the failure mode below.

### Real incident driving this (the actual motivation)

picoserve's monolithic `serve_and_shutdown` future is **one enormous nested-Router
type**. Compiled at `opt-level = "z"`, LLVM minimises code size at the cost of
stack-slot reuse, and a **single request poll measured >161 KiB of stack** — it
**overflowed the Core 0 stack into `.bss` → hardfault** (branch-to-null from a
smashed return address), reproducible whenever LAN traffic kept requests flowing.
Device-verified via the fault-capture handlers in `main.rs` (`sp=0x20056468`, below
`_stack_end=0x20057B98`).

Current fix: pin the firmware to **`opt-level = 2`** (NOT `"z"`) — restores
stack-slot colouring + inlining that collapses picoserve's nesting, cutting the
poll's stack far below the limit. Flash cost (~+150 KiB) is irrelevant against the
2560 KiB region (image only uses ~580 KiB). **Re-measure stack headroom if the
route table or picoserve version changes.**

So the pain is **proven and architectural**, not hypothetical RAM-budgeting. The
giant-poll-stack-frame fragility is a direct consequence of picoserve's nested
future — exactly what nanofish's flat-`match` server avoids. Caveat: the
`opt-level = 2` fix already de-fangs it, so this is motivation, not an emergency.

## Migration delta (the honest part)

What `kiln-app/src/server.rs` actually leans on today, and what nanofish gives:

| picoserve feature kiln uses | sites | nanofish equivalent |
|---|---|---|
| `State` extractor | ~40 | none — pass state into the handler struct, read by hand |
| `Json` extractor / responder | ~10 | none — parse/serialize manually into the buffer |
| `parse_path_segment` (typed path params) | 3 | none — split `req.path` yourself |
| `ChunkedResponse` / `ChunkWriter` / `Chunks` streaming | profiles list + logs (`server.rs:1304-1395`, `html.rs`) | **NONE — confirmed blocker** (see Streaming finding) |
| `AppBuilder` router + blanket OPTIONS handler | core wiring | hand-rolled `match`, write OPTIONS/CORS by hand |

Translation: nanofish is a **lower-level** server, not a modern picoserve. You'd
trade axum-style ergonomics for a manual `match` and **rewrite all routing,
extraction, JSON, and especially the chunked streaming** (profiles + log tail).
TLS story is identical (both HTTP-only → reverse proxy), so no win there.

## Streaming finding (resolved — the deciding factor)

Checked nanofish source (`src/response.rs`, `src/server.rs`, v0.12 / `main`):

```rust
pub enum ResponseBody<'a> {
    Text(&'a str),     // one borrowed slice
    Binary(&'a [u8]),  // one borrowed slice
    Empty,
}
```

Server write path: `response.build_bytes::<>()` → one `response_bytes` buffer →
`socket.write_all(&response_bytes).await` → `flush()`. **Single Content-Length
buffer, written all at once. No chunked Transfer-Encoding, no incremental body, no
chunk writer.** The body must be **fully materialised in one caller-owned buffer
before send.**

That is the exact opposite of what the kiln's chunked paths exist for. `html.rs`
streams `prefix + rendered profile list + suffix` *specifically to avoid buffering
the whole list*; the log tail does the same. On nanofish you'd have to size a
static buffer for the **worst-case full log tail / profiles HTML** and hold it in
RAM — which **eats the no-alloc RAM win that was the whole reason to look at
nanofish.** Self-defeating.

→ **nanofish is a hard no for the kiln** unless we drop streaming or fork it to add
chunked support. Not worth it.

## Verdict

**Don't migrate.** Not "not now" — **not with the current streaming design.**
nanofish can't stream (finding above), so the log tail + profiles list would each
need a worst-case static buffer, cancelling the only RAM upside. Plus days
rewriting 60+ `State`/`Json`/`parse_path_segment` sites for a *less* ergonomic API.
The picoserve stack overflow that motivated this is already fixed by `opt-level=2`.

If the picoserve RAM / stack-frame pain ever forces a move, the candidate is
**`edge-http` / `edge-net`**, NOT nanofish — it's no-alloc *and* supports streaming
+ a full protocol suite, an actual picoserve replacement. Cheapest lever first
though: tune picoserve worker count (`server.rs:60`, `api.rs:16`).

## Open questions before any move

- [x] Does nanofish support response **streaming** / chunked transfer at all? → **NO** (single Content-Length buffer; confirmed in source). Decisive blocker.
- [ ] ~~Measured RAM: nanofish handler + buffers vs picoserve N×84 KB workers.~~ Moot — no-stream forces worst-case buffers, killing the RAM win.
- [ ] **Per-poll stack frame** at `opt-level = "z"` — does nanofish's flat handler stay bounded where picoserve hit >161 KiB? (would let us drop back to `"z"` and reclaim ~150 KiB flash)
- [ ] Concurrent-connection model — does it match picoserve's worker pool?

## Sources

- https://crates.io/crates/nanofish
- https://docs.rs/nanofish
- https://github.com/rttfd/nanofish
- https://lib.rs/crates/edge-http
