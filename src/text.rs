//! Bitmap text at exact pixel sizes from a shared A8 glyph atlas.
//!
//! Startup: fonts load and every glyph the fixed UI styles need is rasterised on a background
//! thread while the main thread brings up SDL/GL; the atlas then fills with one upload. The
//! packed atlas is kept on disk, so later starts read it back instead of rasterising again.
//! Later glyphs (app titles in odd scripts, rare sizes) rasterise lazily as tiny uploads.
use crate::color::Rgba;
use crate::gl;
use crate::render::{Rect, Renderer, Texture};
use std::collections::HashMap;
use std::path::PathBuf;
use std::thread::JoinHandle;

const ATLAS_W: i32 = 2048;
const ATLAS_H: i32 = 2048;
const PAD: i32 = 1;
const FONT_FILES: [&str; 3] = ["Outfit-Regular.ttf", "Outfit-Medium.ttf", "Outfit-SemiBold.ttf"];

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Weight {
    Regular = 0,
    Medium = 1,
    SemiBold = 2,
}

#[derive(Clone, Copy, Debug)]
pub struct Style {
    pub weight: Weight,
    pub size: f32,
    /// Letter spacing in em.
    pub tracking: f32,
}

impl Style {
    pub const fn new(weight: Weight, size: f32) -> Self {
        Self { weight, size, tracking: 0.0 }
    }

    pub const fn tracking(self, tracking: f32) -> Self {
        Self { tracking, ..self }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    weight: Weight,
    size: u16,
    ch: char,
}

impl Key {
    fn new(style: Style, ch: char) -> Self {
        Self { weight: style.weight, size: style.size.round() as u16, ch }
    }
}

#[derive(Clone, Copy)]
struct Glyph {
    uv: [f32; 4],
    w: f32,
    h: f32,
    xmin: f32,
    ymin: f32,
    advance: f32,
}

struct Raster {
    key: Key,
    metrics: fontdue::Metrics,
    bitmap: Vec<u8>,
}

/// A glyph's place in the atlas and its metrics, as kept in the disk cache.
#[derive(Clone, Copy)]
struct Placed {
    key: Key,
    x: u16,
    y: u16,
    w: u16,
    h: u16,
    xmin: f32,
    ymin: f32,
    advance: f32,
}

/// A packed atlas: glyph places and the used rows of pixels.
struct Packed {
    placed: Vec<Placed>,
    cursor: (i32, i32),
    row_h: i32,
    pixels: Vec<u8>,
}

enum Glyphs {
    Fresh(Vec<Raster>),
    Cached(Packed),
}

/// Fonts plus pre-rasterised glyphs, produced off the main thread.
pub struct Prewarmed {
    fonts: Vec<fontdue::Font>,
    glyphs: Glyphs,
    /// Where (and under which key) a freshly packed atlas is saved for the next start.
    cache: Option<(PathBuf, u64)>,
}

impl Prewarmed {
    pub fn from_cache(&self) -> bool {
        matches!(self.glyphs, Glyphs::Cached(_))
    }
}

/// Loads the fonts and rasterises `jobs` (style + the characters it will draw) on a thread,
/// or reads back the atlas an earlier start packed for the same jobs from `cache`.
pub fn prewarm(fonts_dir: PathBuf, jobs: Vec<(Style, String)>, cache: Option<PathBuf>) -> JoinHandle<Result<Prewarmed, String>> {
    std::thread::Builder::new()
        .name("glyphs".into())
        .spawn(move || {
            let font_bytes = FONT_FILES
                .iter()
                .map(|file| std::fs::read(fonts_dir.join(file)).map_err(|e| format!("{file}: {e}")))
                .collect::<Result<Vec<_>, String>>()?;
            let key = cache_key(&jobs, &font_bytes);
            let fonts = font_bytes
                .into_iter()
                .zip(FONT_FILES)
                .map(|(bytes, file)| fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).map_err(|e| format!("{file}: {e}")))
                .collect::<Result<Vec<_>, String>>()?;
            if let Some(packed) = cache.as_deref().and_then(|path| read_cache(path, key)) {
                return Ok(Prewarmed { fonts, glyphs: Glyphs::Cached(packed), cache: None });
            }
            let mut seen = std::collections::HashSet::new();
            let mut rasters = Vec::new();
            for (style, chars) in jobs {
                for ch in chars.chars() {
                    let key = Key::new(style, ch);
                    if seen.insert(key) {
                        let (metrics, bitmap) = fonts[key.weight as usize].rasterize(ch, key.size as f32);
                        rasters.push(Raster { key, metrics, bitmap });
                    }
                }
            }
            // Tallest first packs shelves tightly.
            rasters.sort_by_key(|r| std::cmp::Reverse(r.metrics.height));
            Ok(Prewarmed { fonts, glyphs: Glyphs::Fresh(rasters), cache: cache.map(|path| (path, key)) })
        })
        .expect("glyph thread")
}

// Cache file: magic, key, cursor x/y, shelf height, pixel rows, glyph count (u32 each, little
// endian), the glyphs, then the used atlas rows.
const CACHE_MAGIC: &[u8; 4] = b"VGA1";
const CACHE_HEADER: usize = 32;
const PLACED_BYTES: usize = 27;

/// Changes with the characters and sizes to prewarm, the font files and the build.
fn cache_key(jobs: &[(Style, String)], fonts: &[Vec<u8>]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    env!("CARGO_PKG_VERSION").hash(&mut hasher);
    (ATLAS_W, ATLAS_H, PAD).hash(&mut hasher);
    for (style, chars) in jobs {
        (style.weight as u8, style.size.round() as u16, chars).hash(&mut hasher);
    }
    fonts.hash(&mut hasher);
    hasher.finish()
}

fn read_cache(path: &std::path::Path, key: u64) -> Option<Packed> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() < CACHE_HEADER || &bytes[..4] != CACHE_MAGIC || bytes[4..12] != key.to_le_bytes() {
        return None;
    }
    let u16_at = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
    let u32_at = |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let f32_at = |at: usize| f32::from_bits(u32_at(at));
    let (rows, count) = (u32_at(24) as usize, u32_at(28) as usize);
    let pixels_at = CACHE_HEADER + count * PLACED_BYTES;
    if rows > ATLAS_H as usize || bytes.len() != pixels_at + rows * ATLAS_W as usize {
        return None;
    }
    let mut placed = Vec::with_capacity(count);
    for i in 0..count {
        let at = CACHE_HEADER + i * PLACED_BYTES;
        let weight = match bytes[at] {
            0 => Weight::Regular,
            1 => Weight::Medium,
            _ => Weight::SemiBold,
        };
        placed.push(Placed {
            key: Key { weight, size: u16_at(at + 1), ch: char::from_u32(u32_at(at + 3))? },
            x: u16_at(at + 7),
            y: u16_at(at + 9),
            w: u16_at(at + 11),
            h: u16_at(at + 13),
            xmin: f32_at(at + 15),
            ymin: f32_at(at + 19),
            advance: f32_at(at + 23),
        });
    }
    Some(Packed {
        placed,
        cursor: (u32_at(12) as i32, u32_at(16) as i32),
        row_h: u32_at(20) as i32,
        pixels: bytes[pixels_at..].to_vec(),
    })
}

/// Written on a thread of its own (temp file + rename), so startup never waits for it.
fn write_cache(path: PathBuf, key: u64, packed: Packed) {
    let _ = std::thread::Builder::new().name("glyph-cache".into()).spawn(move || {
        let mut out = Vec::with_capacity(CACHE_HEADER + packed.placed.len() * PLACED_BYTES + packed.pixels.len());
        out.extend_from_slice(CACHE_MAGIC);
        out.extend_from_slice(&key.to_le_bytes());
        let rows = packed.pixels.len() / ATLAS_W as usize;
        for v in [packed.cursor.0 as u32, packed.cursor.1 as u32, packed.row_h as u32, rows as u32, packed.placed.len() as u32] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for p in &packed.placed {
            out.push(p.key.weight as u8);
            out.extend_from_slice(&p.key.size.to_le_bytes());
            out.extend_from_slice(&(p.key.ch as u32).to_le_bytes());
            for v in [p.x, p.y, p.w, p.h] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            for v in [p.xmin, p.ymin, p.advance] {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        out.extend_from_slice(&packed.pixels);
        let temp = path.with_extension("tmp");
        if std::fs::write(&temp, &out).and_then(|()| std::fs::rename(&temp, &path)).is_err() {
            let _ = std::fs::remove_file(&temp);
        }
    });
}

pub struct Text {
    fonts: Vec<fontdue::Font>,
    atlas: Texture,
    cache: HashMap<Key, Glyph>,
    cursor: (i32, i32),
    row_h: i32,
    pub rasterised: u32,
}

impl Text {
    pub fn new(renderer: &Renderer, prewarmed: Prewarmed) -> Self {
        let Prewarmed { fonts, glyphs, cache } = prewarmed;
        let mut text = Self {
            fonts,
            atlas: renderer.create_texture(ATLAS_W, ATLAS_H, gl::ALPHA),
            cache: HashMap::new(),
            cursor: (PAD, PAD),
            row_h: 0,
            rasterised: 0,
        };
        let packed = match glyphs {
            Glyphs::Cached(packed) => packed,
            Glyphs::Fresh(rasters) => {
                // Pack everything into a CPU copy; the used rows upload once below.
                let mut pixels = vec![0u8; (ATLAS_W * ATLAS_H) as usize];
                let mut placed = Vec::with_capacity(rasters.len());
                for raster in &rasters {
                    let m = &raster.metrics;
                    let Some((x, y)) = text.allocate(m.width as i32, m.height as i32) else { break };
                    for row in 0..m.height {
                        let dst = ((y as usize + row) * ATLAS_W as usize) + x as usize;
                        pixels[dst..dst + m.width].copy_from_slice(&raster.bitmap[row * m.width..][..m.width]);
                    }
                    placed.push(Placed {
                        key: raster.key,
                        x: x as u16,
                        y: y as u16,
                        w: m.width as u16,
                        h: m.height as u16,
                        xmin: m.xmin as f32,
                        ymin: m.ymin as f32,
                        advance: m.advance_width,
                    });
                }
                let used = (text.cursor.1 + text.row_h + PAD).min(ATLAS_H);
                pixels.truncate((ATLAS_W * used) as usize);
                let packed = Packed { placed, cursor: text.cursor, row_h: text.row_h, pixels };
                if let Some((path, key)) = cache {
                    write_cache(path, key, Packed { placed: packed.placed.clone(), pixels: packed.pixels.clone(), ..packed });
                }
                packed
            }
        };
        text.cache.reserve(packed.placed.len() + 256);
        for p in &packed.placed {
            text.place(p);
        }
        text.cursor = packed.cursor;
        text.row_h = packed.row_h;
        let rows = (packed.pixels.len() / ATLAS_W as usize) as i32;
        renderer.upload(text.atlas, 0, 0, ATLAS_W, rows, gl::ALPHA, &packed.pixels);
        text
    }

    fn place(&mut self, p: &Placed) {
        let (x, y, w, h) = (p.x as f32, p.y as f32, p.w as f32, p.h as f32);
        let (aw, ah) = (ATLAS_W as f32, ATLAS_H as f32);
        let glyph = Glyph { uv: [x / aw, y / ah, (x + w) / aw, (y + h) / ah], w, h, xmin: p.xmin, ymin: p.ymin, advance: p.advance };
        self.cache.insert(p.key, glyph);
    }

    pub fn glyph_count(&self) -> usize {
        self.cache.len()
    }

    /// Shelf allocation; None when the atlas is full.
    fn allocate(&mut self, w: i32, h: i32) -> Option<(i32, i32)> {
        if self.cursor.0 + w + PAD > ATLAS_W {
            self.cursor = (PAD, self.cursor.1 + self.row_h + PAD);
            self.row_h = 0;
        }
        if self.cursor.1 + h + PAD > ATLAS_H {
            return None;
        }
        let position = self.cursor;
        self.cursor.0 += w + PAD;
        self.row_h = self.row_h.max(h);
        Some(position)
    }

    fn insert(&mut self, key: Key, metrics: &fontdue::Metrics, x: i32, y: i32) -> Glyph {
        let (w, h) = (metrics.width as i32, metrics.height as i32);
        let glyph = Glyph {
            uv: [
                x as f32 / ATLAS_W as f32,
                y as f32 / ATLAS_H as f32,
                (x + w) as f32 / ATLAS_W as f32,
                (y + h) as f32 / ATLAS_H as f32,
            ],
            w: w as f32,
            h: h as f32,
            xmin: metrics.xmin as f32,
            ymin: metrics.ymin as f32,
            advance: metrics.advance_width,
        };
        self.cache.insert(key, glyph);
        glyph
    }

    fn glyph(&mut self, renderer: &Renderer, key: Key) -> Glyph {
        if let Some(glyph) = self.cache.get(&key) {
            return *glyph;
        }
        let font = &self.fonts[key.weight as usize];
        // Characters the font lacks (other scripts) are skipped rather than drawn as boxes.
        if font.lookup_glyph_index(key.ch) == 0 && !key.ch.is_whitespace() {
            let empty = Glyph { uv: [0.0; 4], w: 0.0, h: 0.0, xmin: 0.0, ymin: 0.0, advance: 0.0 };
            self.cache.insert(key, empty);
            return empty;
        }
        let (metrics, bitmap) = font.rasterize(key.ch, key.size as f32);
        let (w, h) = (metrics.width as i32, metrics.height as i32);
        let (x, y) = match self.allocate(w, h) {
            Some(position) => position,
            None => {
                // Full: start over. Glyphs re-rasterise lazily; quads queued this frame may
                // briefly show a wrong glyph, which the next frame corrects.
                self.cache.clear();
                self.cursor = (PAD, PAD);
                self.row_h = 0;
                self.allocate(w, h).unwrap_or((PAD, PAD))
            }
        };
        if w > 0 && h > 0 {
            renderer.upload(self.atlas, x, y, w, h, gl::ALPHA, &bitmap);
        }
        self.rasterised += 1;
        self.insert(key, &metrics, x, y)
    }

    pub fn measure(&mut self, renderer: &Renderer, text: &str, style: Style) -> f32 {
        let size = style.size.round();
        let mut width = 0.0;
        let mut previous = None;
        for ch in text.chars() {
            if let Some(prev) = previous {
                width += self.fonts[style.weight as usize].horizontal_kern(prev, ch, size).unwrap_or(0.0);
            }
            width += self.glyph(renderer, Key::new(style, ch)).advance + style.tracking * size;
            previous = Some(ch);
        }
        width - if previous.is_some() { style.tracking * size } else { 0.0 }
    }

    /// Ascent above the baseline in px, for placing text by its top edge.
    pub fn ascent(&self, style: Style) -> f32 {
        self.fonts[style.weight as usize]
            .horizontal_line_metrics(style.size.round())
            .map_or(style.size * 0.8, |m| m.ascent)
    }

    /// Draws `text` with its baseline at `y`. Returns the advance width.
    pub fn draw(&mut self, renderer: &mut Renderer, text: &str, x: f32, y: f32, style: Style, color: Rgba) -> f32 {
        let size = style.size.round();
        let mut pen = x;
        let mut previous = None;
        for ch in text.chars() {
            if let Some(prev) = previous {
                pen += self.fonts[style.weight as usize].horizontal_kern(prev, ch, size).unwrap_or(0.0);
            }
            let glyph = self.glyph(renderer, Key::new(style, ch));
            if glyph.w > 0.0 {
                let gx = (pen + glyph.xmin).round();
                let gy = (y - glyph.ymin - glyph.h).round();
                renderer.glyph(self.atlas, Rect::new(gx, gy, glyph.w, glyph.h), glyph.uv, color);
            }
            pen += glyph.advance + style.tracking * size;
            previous = Some(ch);
        }
        pen - x
    }

    /// Draws text by its top edge (CSS-like placement).
    pub fn draw_top(&mut self, renderer: &mut Renderer, text: &str, x: f32, top: f32, style: Style, color: Rgba) -> f32 {
        let baseline = top + self.ascent(style);
        self.draw(renderer, text, x, baseline, style, color)
    }
}
