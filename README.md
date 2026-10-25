# Vision

Ein schneller, nativer Home-Launcher für gerootete LG-webOS-TVs (entwickelt auf einem LG CX, webOS 5).
Rust + SDL2 + eigener OpenGL-ES-2-Renderer. Design: Richtung A „Cinematic Ambient“.

**Performance hat Vorrang vor allem anderen.** Gemessen auf dem CX (Mali-G51):

| Messwert | Wert |
|---|---|
| Kaltstart bis erstes Bild | ~410 ms |
| App-Liste (Luna, direkt) | ~7 ms |
| CPU-Arbeit pro Frame beim Navigieren | ~1 ms (max. ~4 ms) |
| Bildrate | 50 fps (= UI-Takt des TVs), stabil |
| GPU im Leerlauf | 0 % (es wird nur bei Änderungen gezeichnet) |
| Draw-Calls pro Frame | 1–3 |

## Wie es schnell bleibt

- **Nur zeichnen, wenn sich etwas ändert:** Der UI-Thread schläft in `SDL_WaitEventTimeout`, bis eine Taste, neue Daten, die nächste Minute (Uhr) oder eine laufende Animation ihn weckt.
- **Ein Shader für alles:** abgerundete Rechtecke, weiche Schatten, Glow, Ambient-Licht und Rahmen sind analytische SDFs im Fragment-Shader. Es gibt keine Offscreen-Passes und keinen Echtzeit-Blur.
- **Gebündeltes Zeichnen:** Ein gestreamter Vertex-Buffer, Texturwechsel nur zwischen den Atlanten (Formen → Icons → Glyphen).
- **Text als Bitmap-Atlas:** Jede Glyphe wird pro Größe einmal gerastert (fontdue) und als kleines Sub-Image hochgeladen.
- **Kein Warten im UI-Thread:** Luna-Aufrufe, Dateizugriffe und PNG-Dekodierung laufen im Plattform-Worker.
- **Nichts wird beim Build gelinkt:** SDL, GLES, libluna-service2 und GLib werden zur Laufzeit per `dlopen` geladen. So lässt sich das Projekt ohne TV-Sysroot cross-kompilieren.

## Aufbau

```
src/main.rs      Event-Loop, Fenster/GL-Kontext, Frame-Timing
src/render.rs    GLES2-Batch-Renderer (SDF-Shader)
src/text.rs      Glyphen-Atlas (fontdue)
src/home.rs      Home-Screen: Zustand, Fokus, Zeichnen
src/theme.rs     Design-Tokens (Richtung A „Cinematic Ambient“)
src/anim.rs      kritisch gedämpfte Federn
src/platform.rs  Worker: Desktop-Beispieldaten bzw. TV-Dienste
src/luna.rs      Luna-Client (libluna-service2 + GLib, nur ARM)
src/model.rs     Apps, Eingänge, Icon-Dekodierung und Farbanalyse
```

## Entwickeln

Voraussetzungen: Rust (MSVC-Host), Target `armv7-unknown-linux-gnueabi`, [Zig](https://ziglang.org) 0.14 und `cargo-zigbuild` im PATH, Node.js, PuTTY (`plink`/`pscp`), `gh`.

```sh
sh scripts/fetch-deps.sh        # SDL-Bibliotheken holen (einmalig)
node scripts/make-icons.mjs     # App-Icons erzeugen (einmalig)

cargo run                       # Desktop-Vorschau mit Beispieldaten
cargo run -- --frames 120 --keys right,right,down --screenshot shot.png

node scripts/package.mjs        # TV-Build (ARMv7 softfp, glibc 2.28) + IPK nach out/
node scripts/tv.mjs install     # aufs TV laden und installieren
node scripts/tv.mjs launch      # starten
node scripts/tv.mjs bench       # Navigation per Skript + Frame-Zeiten im Log
node scripts/tv.mjs log         # /tmp/vision.log anzeigen
node scripts/tv.mjs shot        # Screenshot der UI-Ebene nach out/tv.jpg
```

TV-Zugang über `VISION_TV_HOST` / `VISION_TV_PASSWORD` (Standard: `192.168.178.96` / `alpine`).

## Stand

Phase 1 (Gerüst) ist fertig: echte App-Liste mit Icons in LG-Reihenfolge, HDMI-Eingänge, Ambient-Farbe aus dem Icon, Start über Luna. Die App läuft als Dev-App und übernimmt die Home-Taste noch nicht.

Nächste Schritte: Icon-Cache auf Platte (spart den Homebrew-Kaltstart von ~3 s), Favoriten-Editor, Wetter, Home Assistant, Plex „Weiterschauen“, TiviOne (opt-in), danach Home-Takeover.

## Lizenzen von Drittanbietern

- SDL2 / SDL-webOS: zlib-Lizenz (`licenses/SDL2.txt` im IPK)
- Outfit (Schrift): SIL Open Font License 1.1 (`assets/fonts/OFL.txt`)
- Die Integration in webOS orientiert sich an [Luma](https://github.com/ecklf/lg-luma) (MIT), es wurde kein Code übernommen.
