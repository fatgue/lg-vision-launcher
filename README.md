# Vision

A fast, native home launcher for rooted LG webOS TVs (developed on an LG CX, webOS 5).
Rust + SDL2 + a custom OpenGL ES 2 renderer. Design: direction A, "Cinematic Ambient".

**Performance comes first.** Measured on the CX (Mali-G51):

| Metric | Value |
|---|---|
| Cold start to first frame | ~410 ms |
| App list (Luna, direct) | ~7 ms |
| CPU work per frame while navigating | ~1 ms (max ~4 ms) |
| Frame rate | 50 fps (the TV's UI refresh), steady |
| GPU when idle | 0 % (draws only when something changes) |
| Draw calls per frame | 1–3 |

## How it stays fast

- **Render on demand:** the UI thread sleeps in `SDL_WaitEventTimeout` until a key, new data, the next minute (clock) or a running animation wakes it.
- **One shader for everything:** rounded rects, soft shadows, glows, ambient light and strokes are analytic SDFs in the fragment shader. No offscreen passes, no real-time blur.
- **Batched drawing:** one streamed vertex buffer; textures only switch between atlases (shapes → icons → glyphs).
- **Bitmap glyph atlas:** each glyph is rasterised once per size (fontdue) and uploaded as a tiny sub-image.
- **Nothing blocks the UI thread:** Luna calls, file reads and PNG decoding run on the platform worker.
- **Nothing is linked at build time:** SDL, GLES, libluna-service2 and GLib are loaded with `dlopen`, so the project cross-compiles without a TV sysroot.

## Layout

```
src/main.rs      event loop, window/GL context, frame timing
src/render.rs    GLES2 batch renderer (SDF shader)
src/text.rs      glyph atlas (fontdue)
src/home.rs      home screen: state, focus, drawing
src/theme.rs     design tokens (direction A, "Cinematic Ambient")
src/anim.rs      critically damped springs
src/platform.rs  worker: desktop sample data or TV services
src/luna.rs      Luna client (libluna-service2 + GLib, ARM only)
src/model.rs     apps, inputs, icon decoding and colour analysis
```

## Development

Requirements: Rust (MSVC host), target `armv7-unknown-linux-gnueabi`, [Zig](https://ziglang.org) 0.14 and `cargo-zigbuild` on `PATH`, Node.js, PuTTY (`plink`/`pscp`), `gh`.

```sh
sh scripts/fetch-deps.sh        # fetch the SDL libraries (once)
node scripts/make-icons.mjs     # generate the app icons (once)

cargo run                       # desktop preview with sample data
cargo run -- --frames 120 --keys right,right,down --screenshot shot.png

node scripts/package.mjs        # TV build (ARMv7 softfp, glibc 2.28) + IPK in out/
node scripts/tv.mjs install     # upload to the TV and install
node scripts/tv.mjs launch      # start it
node scripts/tv.mjs bench       # scripted navigation, frame timings in the log
node scripts/tv.mjs log         # show /tmp/vision.log
node scripts/tv.mjs shot        # capture the UI plane to out/tv.jpg
```

TV access via `VISION_TV_HOST` / `VISION_TV_PASSWORD` (defaults: `192.168.178.96` / `alpine`).

## Status

Phase 1 (scaffold) is done: the real app list with icons in LG's order, HDMI inputs, ambient colour taken from the focused icon, launching via Luna. Vision runs as a dev app and does not take over the Home key yet.

Next: on-disk icon cache (avoids the ~3 s Homebrew service cold start), favourites editor, weather, Home Assistant, Plex "Continue watching", TiviOne (opt-in), then the Home takeover.

## Third-party licences

- SDL2 / SDL-webOS: zlib licence (`licenses/SDL2.txt` in the IPK)
- Outfit (typeface): SIL Open Font License 1.1 (`assets/fonts/OFL.txt`)
- The webOS integration follows the approach of [Luma](https://github.com/ecklf/lg-luma) (MIT); no code was copied.
