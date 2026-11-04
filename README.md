<p align="center">
  <img src="docs/media/visi.png" width="240" alt="Visi, Vision's mascot">
</p>

<h1 align="center">Vision</h1>

<p align="center">
  A fast, native home launcher for rooted LG webOS TVs (developed on an LG CX, webOS 5).<br>
  Rust + SDL2 + a custom OpenGL ES 2 renderer, around 3 MB, no web view.<br>
  <sub>Say hello to <b>Visi</b>, Vision's little drone: <a href="#meet-visi">leave your TV alone for a while and it shows up</a>.</sub>
</p>

> **🍺 Homebrew app store built in.** Browse every app of the webOS Homebrew repository right from
> your home screen, and install, update or uninstall it with one click. No PC, no Dev Manager.
> [See how it works ↓](#homebrew-app-store)

![Vision's home screen](docs/screenshots/home.jpg)

| | |
|---|---|
| ![TiviOne on the home screen](docs/screenshots/tivione-tile.jpg) | ![Settings](docs/screenshots/settings.jpg) |

![TiviOne's page inside Vision](docs/screenshots/tivione-store.jpg)

## Features

- **Homebrew app store:** all apps of the webOS Homebrew repository with icons, details and updates; install and uninstall them on the TV ([see below](#homebrew-app-store)).
- **Your apps, your order:** LG's apps with full-tile artwork, favourites you can move and hide, HDMI inputs with signal state.
- **Vision as Home:** the Home key opens Vision instead of LG's launcher, in about 50 ms (see below).
- **Frees memory:** as Home, Vision closes the app that has been in the background longest when the TV runs low on memory (see below).
- **Ambient light** taken from the focused tile, a large hero with what is focused, clock and weather.
- **Home Assistant:** lights, switches and scenes, grouped by room: the focused room's devices appear in the row below it, and OK on a room turns all its lights on or off. Hold OK on a room or device to move it. Set up from the phone via QR code.
- **Plex:** continue watching, linked through plex.tv/link.
- **TiviOne (optional):** with the TiviOne IPTV app installed and connected, Vision shows recent and favourite channels, catch-up, upcoming reminders and a muted live preview of the focused channel; the number keys tune channels.
- **Six languages:** English, German, French, Spanish, Italian, Turkish (follows the TV, can be changed).
- **Magic Remote:** point to focus, click to open, hold for options, wheel to step along a row.
- **Two looks:** Cinematic and Glass (see below), switchable in Settings while Vision runs.
- **Intro:** when the TV starts, a short cinematic intro: "VISION" in steel with depth, a sweeping light and an anamorphic flare, drawn live (any key skips it; Settings → Appearance → Intro on start).
- **[Visi](#meet-visi), the drone:** leave Home alone for a few minutes and a small drone flies in and wrecks the place: it knocks tiles off the dock and juggles them, lasers a hole into the screen and flies through it, burns the clock away, cracks the TV's glass, spins up a tornado, and more, a different show every time. Tiles and their pieces fall with real physics. Then it rewinds everything and leaves; nothing really changes, and any key ends the show at once (Settings → Appearance: off, 2, 5 or 10 minutes).

## Homebrew app store

![Homebrew apps in Vision](docs/screenshots/homebrew.jpg)

Open the **Homebrew** tile among your apps and you get the whole
[webOS Homebrew repository](https://repo.webosbrew.org) on your TV:

- **Browse** every app with its icon and description, in three tabs: **All**, **Installed** and **Updates**.
- **Install** with one click; **Update** shows up as soon as the repository has a newer version.
- **Open** or **Uninstall** installed apps from the same page (uninstalling asks first).
- **Safe:** Vision checks each package's SHA-256 against the repository before installing it through
  Homebrew Channel. Apps that run with root rights are marked *Root*; before installing or updating
  one, Vision shows a warning and asks twice. Vision and Homebrew Channel themselves cannot be
  uninstalled here.
- **Extras:** [own-your-glass](https://github.com/JulioFerrero/own-your-glass) (hardens the TV: microphones,
  screen capture, ads, telemetry) is listed too, installed from its GitHub releases. It needs webOS 10
  or newer; on older TVs it shows greyed out with a note and cannot be installed.
- The tile behaves like any app: move it or hide it with **Hold OK**.

![An app's page with the Install button](docs/screenshots/homebrew-app.jpg)

## Themes

A theme changes the whole home screen: layout, navigation, typography, focus effects and the
settings screens, not just colours. Switch it in **Settings → Appearance → Theme**.

**Cinematic:** a large title over ambient light from the focused tile, with rows of apps, channels,
Home Assistant tiles and inputs.

![Cinematic: moving through the rows into Settings](docs/media/cinematic.gif)

**Glass:** a photo wallpaper with the first row of apps in a frosted glass dock and more rows below
it. At the top left a large clock with the date, the weather and a five-day forecast (or date and time
in one line: **Settings → Clock → Clock style**); at the top right two round glass buttons for the
inputs and the **Control Center**. A focused app lifts, with its name underneath; apps wiggle while
you move them. When TiviOne or Plex has focus in the dock, its channels or "continue watching" appear
above the dock, and Up opens them with programme details and the live preview.

- **Control Center:** the TV's sound and picture mode (the picture modes that suit what is on screen:
  SDR, HDR or Dolby Vision), the look, the sleep timer, screen off, turning the TV off, the next
  wallpaper, Settings, and the brightness (Left / Right).
- **Settings:** full screen over the blurred wallpaper, a large glass card with each section's symbol
  and what it is for, and the list in glass rows.
- Three wallpapers come with Vision, or send your own photo from your phone (**Settings → Appearance →
  Photo from your phone**). The glass is a blurred copy of the wallpaper seen through the shapes, made
  once: nothing is blurred while drawing. Glass uses the Inter typeface and Material Symbols.

![Glass: the dock, the Top Shelf, rooms and devices, the Control Center and Settings](docs/media/glass.gif)

| | |
|---|---|
| ![Glass home](docs/screenshots/glass-home.jpg) | ![The Control Center](docs/screenshots/glass-control.jpg) |
| ![Glass Top Shelf](docs/screenshots/glass-shelf.jpg) | ![Glass settings](docs/screenshots/glass-settings.jpg) |

## Performance

Performance comes first. Measured on the CX (Mali-G51):

| Metric | Value |
|---|---|
| Cold start to first frame | ~250 ms (~430 ms on the first start after an update) |
| Home key to Vision (already running) | ~50 ms |
| CPU work per frame while navigating | ~1 ms (max ~4 ms) |
| CPU work per frame during Visi's show | ~2–3.5 ms |
| Frame rate | 50 fps (the TV's UI refresh), steady |
| GPU when idle | 0 % (draws only when something changes) |
| Draw calls per frame | 1–3 |

How it stays fast:

- **Render on demand:** the UI thread sleeps until a key, new data, the next minute or a running animation wakes it.
- **One shader for everything:** rounded rects, soft shadows, glows and strokes are analytic SDFs. No offscreen passes, no real-time blur.
- **Batched drawing** from texture atlases. Glyphs are rasterised once on a background thread; the packed atlas is kept on disk and read back on later starts.
- **Nothing blocks the UI thread:** Luna calls, network requests, file access and image decoding run on workers.
- **Nothing is linked at build time:** SDL, GLES, libluna-service2 and GLib are loaded with `dlopen`.

## Installing

Vision needs a **rooted** LG webOS TV with [Homebrew Channel](https://github.com/webosbrew/webos-homebrew-channel)
(root is used for the app list and artwork, switching the Home key and installing apps).

1. Download the `.ipk` from the [latest release](https://github.com/fatgue/lg-vision-launcher/releases).
2. Install it with Homebrew Channel or the webOS Dev Manager.
3. Open Vision. To make it your Home launcher: **Settings → Home button → Vision as Home**.

**Updates:** Vision looks for a new release on GitHub every six hours. When there is one, it says so,
and **Settings → System → Vision update** installs it (checked against the release's `SHA256SUMS`)
and restarts Vision.

## TiviOne

[TiviOne](https://tivione.zf-labs.de) is an IPTV player for LG TVs by the same developer. It brings its own
features; Vision only shows what TiviOne shares with it, and only after you agree.

- Without TiviOne, Vision shows one TiviOne tile under *Live TV*. **OK** opens its page with pictures and details
  and an **Install** button; **Hide from Home** (or holding OK on the tile) removes the tile for good.
  **Settings → Connections** always keeps a *TiviOne* entry, and *TiviOne on Home* brings the tile back.
- **Install** downloads TiviOne from tivione.zf-labs.de. Vision checks the release manifest's Ed25519
  signature and the package's SHA-256 before installing it through Homebrew Channel. For a TiviOne installed
  this way, Vision offers updates the same way; a TiviOne from the LG Content Store updates through LG.
- TiviOne includes no channels: it plays your own IPTV subscription. It is free with 30 minutes of live TV a
  day; Premium is bought inside TiviOne.
- Connecting is opt-in: TiviOne opens once to pair, then shares an encrypted snapshot (channels, recent,
  favourites, guide, catch-up) through a local file. **Settings → Connections → TiviOne** turns it off again.
- **Live preview:** rest on a channel for a moment and it plays, muted, right on the home screen (on the TV's
  video plane, so it costs Vision nothing). **OK** opens it in TiviOne. With a provider that allows only one
  stream, TiviOne waits a moment for the preview to let go (**Settings → Appearance → Preview with a
  one-stream provider** turns the preview off for such providers).

| Glass | Cinematic |
|---|---|
| ![The TiviOne live preview in Glass](docs/media/tivione-glass.gif) | ![The TiviOne live preview in Cinematic](docs/media/tivione-cinematic.gif) |

<sub>Recorded on the desktop build, where nothing plays: the pictures in the preview window stand in for the channels.</sub>

## Meet Visi

<img src="docs/media/visi.png" width="140" align="right" alt="Visi">

Visi is Vision's mascot, a small hover drone with a big lens. Leave the home screen alone for a few
minutes and Visi flies in and makes a mess of it: a different show of eight acts every time, out of these:

- **Shoves** a tile off the dock (it takes its neighbour with it), **grabs** one and drops it,
  **juggles** one with its head, **sucks** one in (and burps).
- **Lasers** a row of tiles, fires a **rocket** that blows a tile into pieces, **headbutts** one in two.
- Spins up a **tornado** that whirls three tiles around and flings them away.
- **Burns the clock** away, **draws a smiley** on the wall with its laser.
- Flies off into the **depth of the room**, behind the tiles, and back out.
- **Lasers a hole** into the screen, flies through it and comes out of another one.
- Takes a run-up and **crashes into the TV's glass**: cracks across the whole picture, and Visi, squashed
  flat, slides down.
- Throws a **disco** with confetti, or spins until it is **dizzy**, drops to the floor, sees stars and reboots.

Loose tiles and their pieces are rigid bodies: they fall, land on their corners, bounce and tip over.
Visi turns in 3D (eight views of the drone, blended as it turns) and tilts into its moves like a real
quadcopter. In the end it **rewinds everything**, and nothing has really changed: the order of your apps
and all data stay as they were. **Any key ends the show at once** (and does nothing else).

| Cinematic | Glass |
|---|---|
| ![Visi in Cinematic: a rocket, the cracked glass, a smiley, juggling](docs/media/visi-cinematic.gif) | ![Visi in Glass: a tornado, disco, the burnt clock, a hole in the screen](docs/media/visi-glass.gif) |

Choose when Visi comes (or turn it off) in **Settings → Appearance → Visi drone when idle**: off, after
2, 5 (default) or 10 minutes. It never comes while a live preview plays, a menu is open or *Motion* is set
to *Reduced*. A show costs about 2–3.5 ms of CPU per frame on the CX, at a steady 50 fps; the rest of the
time it costs nothing.

## Using it

- **Left / Right / Up / Down** move the focus; **OK** opens the app, input or channel.
- **Homebrew:** open the Homebrew tile; **Left / Right** at the top switch between All, Installed and Updates, **OK** opens an app's page.
- **Hold OK** on a favourite: **Move** (Left / Right, OK to finish, Back to cancel) or **Hide**.
  Hidden apps have their own row at the bottom; hold OK there to show them again.
- **Settings:** press **Up** from the top row (or click the gear). In Glass, Up reaches the two round
  buttons at the top right: **Inputs** and the **Control Center**, which holds Settings.
  - Connections: Home Assistant, Home tiles, Plex, TiviOne
  - Weather: show or hide, location (from Home Assistant or a city entered on the phone), °C / °F
  - Clock: 24- or 12-hour, date, clock style (Glass)
  - Home button: Vision as Home on or off
  - Rows: show or hide each row
  - Appearance: theme, wallpaper and a photo from your phone (Glass), ambient light (Cinematic), motion, live preview, intro on start, Visi drone
  - System: Vision update (when there is one), language, closing unused apps when memory is low (as Home), reload app images, version
  - TV: the TV's own settings: Home when the TV turns on, Quick Start+, picture mode, brightness and contrast
    (Left / Right), energy saving, Ultra HD Deep Colour per HDMI input, sound mode and output, sleep timer, auto power
    off, and a way into all of LG's settings. Picture settings apply to the current input and signal, as in LG's menu.

## Vision as Home

On webOS 5 and 6, Vision can take over the Home key:

```sh
sh /media/developer/apps/usr/palm/applications/com.vision.launcher/home.sh enable
sh /var/lib/vision-home/home.sh disable   # back to LG's Home
```

`tv/home.sh` bind-mounts Vision's executable over `/usr/bin/com.webos.app.home`. LG's Home manifest and LS2
role stay untouched and nothing on the read-only system partition changes. Vision registers itself with the
application manager, so the Home key brings the running instance to the front. A Homebrew Channel boot hook
(`64-vision-home`, a symlink) re-applies the mount after a cold boot and does nothing if Vision is gone.

## Freeing memory

webOS keeps apps alive in the background and preloads some (Netflix and YouTube, for example). With a
large app in front, the TV then runs out of memory, swaps, and every app stutters. As Home, Vision
checks the available memory every 5 seconds. Below 450 MB it closes the app that has been in the
background longest, one at a time, until 550 MB are free again. It never closes Vision, the app in
front, an app used in the last two minutes, or system apps (Live TV, inputs, LG's home, voice,
AirPlay, Homebrew Channel); LG's browser and LG Channels may go. Closed apps simply start fresh the
next time. Turn it off in **Settings → System → Close unused apps when memory is low**.

## Network and privacy

Vision talks only to what you set up, plus two public services:

| Service | When | What |
|---|---|---|
| open-meteo.com | weather is on | the location's coordinates |
| Your Home Assistant | connected | states and your actions (LAN) |
| plex.tv / your Plex server | connected | continue watching, artwork |
| tivione.zf-labs.de | at most every 6 hours | the signed TiviOne release manifest (no data is sent) |
| repo.webosbrew.org | the Homebrew page is opened | the app list, icons and the packages you install (no data is sent) |
| github.com | the Homebrew page is opened | the release manifest of the extras (own-your-glass) and their packages |
| api.github.com | at most every 6 hours | Vision's latest release (no data is sent) |

Settings and tokens stay on the TV in `/media/developer/vision`. There is no analytics or telemetry.

## Development

Requirements: Rust (MSVC host), target `armv7-unknown-linux-gnueabi`, [Zig](https://ziglang.org) 0.14 and
`cargo-zigbuild`, Node.js, PuTTY (`plink` / `pscp`).

```sh
sh scripts/fetch-deps.sh        # fetch the SDL libraries (once)
node scripts/make-icons.mjs     # generate the app icons (once)

cargo run                       # desktop preview with sample data
cargo run -- --frames 120 --keys down,ok --screenshot shot.png
                                # (`wait` in --keys skips a step; VISION_WEBOS=10 plays a webOS 10 TV)
cargo run -- --visi --visi-seed 7 # Visi's show at once (the same show for the same seed)
cargo run -- --frames 600 --record frames/ 4   # every 4th frame as a PNG (the README's animations)

node scripts/package.mjs        # TV build (ARMv7 softfp, glibc 2.28) + IPK in out/
node scripts/tv.mjs install     # upload to the TV and install
node scripts/tv.mjs launch      # start it
node scripts/tv.mjs bench       # scripted navigation, frame timings in the log
node scripts/tv.mjs log         # show the log
node scripts/tv.mjs shot        # capture the UI plane to out/tv.jpg
```

Set the TV with `VISION_TV_HOST` and `VISION_TV_PASSWORD`.

**Releases:** bump the version in `Cargo.toml` and `appinfo.json`, then push a tag `vX.Y.Z`. GitHub Actions
(`.github/workflows/build.yml`) builds the package from that tag and attaches it, with `SHA256SUMS`, to the release.
Every push to `main` builds it too (download it from the workflow run).

```
src/main.rs       event loop, window and GL context, frame timing
src/render.rs     GLES2 batch renderer (SDF shader)
src/text.rs       glyph atlas (fontdue)
src/home/         home screen: state and focus (mod.rs), the Cinematic and Glass looks,
                  navigation, settings and pages (TiviOne, Homebrew), Visi (visi.rs)
src/theme.rs      design tokens
src/platform.rs   TV services worker (desktop: sample data)
src/services.rs   weather, Home Assistant, Plex, TiviOne snapshot
src/installer.rs  TiviOne release check; download, verification, install and uninstall of apps
src/video.rs      live preview on the TV's video plane
src/luna.rs       Luna client (libluna-service2 + GLib)
src/strings.rs    all user-visible text, six languages
```

## Licence

Vision is released under the [MIT licence](LICENSE). Not affiliated with LG Electronics.

- SDL2 / SDL-webOS: zlib licence (`licenses/SDL2.txt` in the IPK)
- Outfit (typeface): SIL Open Font License 1.1 (`assets/fonts/OFL.txt`)
- Inter (typeface, Glass): SIL Open Font License 1.1 (`assets/fonts/Inter-OFL.txt`), subset to Latin
- Material Symbols Rounded (icons): Apache License 2.0 (`assets/fonts/MaterialSymbols-LICENSE.txt`), subset to the icons Vision uses
- Wallpapers: CC0 photos from Wikimedia Commons (`assets/wallpapers/CREDITS.txt`)
- Visi (`assets/visi.png`, `docs/media/visi.png`): made for Vision with Krea 2 and Qwen-Image-Edit (Apache 2.0), part of Vision under its MIT licence; so are the channel pictures in the preview animations
- The Home takeover follows the approach of [Luma](https://github.com/ecklf/lg-luma) (MIT); no code was copied.
