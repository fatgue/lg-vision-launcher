# Vision

A fast, native home launcher for rooted LG webOS TVs (developed on an LG CX, webOS 5).
Rust + SDL2 + a custom OpenGL ES 2 renderer. Design: direction A, "Cinematic Ambient".

**Performance comes first.** Measured on the CX (Mali-G51):

| Metric | Value |
|---|---|
| Cold start to first frame | ~380 ms |
| App list (Luna, direct) | ~7 ms |
| Icons on later starts (disk cache) | ~24 ms, ready before the first frame |
| CPU work per frame while navigating | ~1 ms (max ~4 ms) |
| Frame rate | 50 fps (the TV's UI refresh), steady |
| GPU when idle | 0 % (draws only when something changes) |
| Draw calls per frame | 1–3 |

## How it stays fast

- **Render on demand:** the UI thread sleeps in `SDL_WaitEventTimeout` until a key, new data, the next minute (clock) or a running animation wakes it.
- **One shader for everything:** rounded rects, soft shadows, glows, ambient light and strokes are analytic SDFs in the fragment shader. No offscreen passes, no real-time blur.
- **Batched drawing:** one streamed vertex buffer; textures only switch between atlases (shapes → icons → glyphs).
- **Glyphs prewarmed off the main thread:** fonts load and every glyph the UI needs (~1,200) rasterises on a background thread while SDL/GL start; the atlas fills with a single upload. Nothing rasterises during navigation.
- **Icon disk cache:** decoded icons (RGBA, ≤128 px, dominant colour) live in the data directory, so the slow Homebrew root path is only needed for apps installed since the last start.
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
src/storage.rs   preferences and icon cache (outside the app payload)
src/strings.rs   all user-visible text (English base, more locales later)
```

Persistent data lives in `/media/developer/vision` on the TV (survives reinstalls):
`prefs.json` (favourites order, hidden apps) and `icons/` (decoded icon cache).

## Using it

- **Left / Right / Up / Down** move the focus; **OK** opens the app or input.
- **Hold OK** on a favourite opens a menu: **Move** (Left / Right, OK to finish, Back to cancel) or **Hide**.
- Hidden apps appear in their own row at the bottom; **hold OK** there to show them again.
- **Settings:** press **Up** from the top row (or click the gear, top right). Sections:
  - Connections: Home Assistant, Home tiles (entities by category and room), Plex, TiviOne
  - Weather: show/hide, location (from Home Assistant or a city entered on the phone), °C/°F
  - Clock: 24-/12-hour, show date
  - Home button: Vision as Home on/off (switches tv/home.sh as root)
  - Rows: show/hide Continue watching, Live TV, Home, Inputs, Hidden apps
  - Appearance: ambient light (off/low/normal/high), motion (full/reduced)
  - System: reload app images, launcher, IP address, version
- **Magic Remote:** point to focus, click to open, hold to open the menu, wheel to step along a row.

## Vision as Home

On webOS 5/6, Vision can replace LG's Home launcher (root required):

```sh
node scripts/tv.mjs home-enable    # or on the TV: sh .../com.vision.launcher/home.sh enable
node scripts/tv.mjs home-status
node scripts/tv.mjs home-disable   # restores LG Home (sh /var/lib/vision-home/home.sh disable)
```

`tv/home.sh` bind-mounts Vision's executable over `/usr/bin/com.webos.app.home`. LG's Home
manifest and LS2 role stay untouched, nothing on the read-only system partition changes. SAM
starts Vision in LG Home's place; Vision registers itself (`registerApp`, lifecycle v2), so the
Home key relaunches the running instance (about 50 ms to the front). A webosbrew boot hook
(`64-vision-home`) re-applies the mount after a cold boot; `scripts/tv.mjs install` refreshes it
after an update. Opening Vision's own tile while it is Home brings the Home instance forward.

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

- Phase 1 (scaffold) is done: the real app list with icons in LG's order, HDMI inputs, ambient colour taken from the focused icon, launching via Luna.
- Phase 2 is done: glyph prewarming, the icon disk cache, the favourites editor (move / hide / show) with persistence, and centralised strings ready for localisation.

- Phase 3 is done: weather, Home Assistant (phone setup via QR code), Plex continue watching (plex.tv/link), Settings row, opt-in TiviOne integration (sealed snapshot, deep links, digit entry).
- Phase 4 is done: Vision is the Home launcher (see above).

Next: localisation (5 languages).

## Third-party licences

- SDL2 / SDL-webOS: zlib licence (`licenses/SDL2.txt` in the IPK)
- Outfit (typeface): SIL Open Font License 1.1 (`assets/fonts/OFL.txt`)
- The webOS integration follows the approach of [Luma](https://github.com/ecklf/lg-luma) (MIT); no code was copied.
