//! Bitmap text at exact pixel sizes from a shared A8 glyph atlas.
//!
//! Startup: fonts load and every glyph the fixed UI styles need is rasterised on a background
//! thread while the main thread brings up SDL/GL; the atlas then fills with one upload. Later
//! glyphs (app titles in odd scripts, rare sizes) rasterise lazily as tiny sub-image uploads.
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

/// Fonts plus pre-rasterised glyphs, produced off the main thread.
pub struct Prewarmed {
    fonts: Vec<fontdue::Font>,
    rasters: Vec<Raster>,
}

/// Loads the fonts and rasterises `jobs` (style + the characters it will draw) on a thread.
pub fn prewarm(fonts_dir: PathBuf, jobs: Vec<(Style, String)>) -> JoinHandle<Result<Prewarmed, String>> {
    std::thread::Builder::new()
        .name("glyphs".into())
        .spawn(move || {
            let fonts = FONT_FILES
                .iter()
                .map(|file| {
                    let bytes = std::fs::read(fonts_dir.join(file)).map_err(|e| format!("{file}: {e}"))?;
                    fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).map_err(|e| format!("{file}: {e}"))
                })
                .collect::<Result<Vec<_>, String>>()?;
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
            Ok(Prewarmed { fonts, rasters })
        })
        .expect("glyph thread")
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
        let mut text = Self {
            fonts: prewarmed.fonts,
            atlas: renderer.create_texture(ATLAS_W, ATLAS_H, gl::ALPHA),
            cache: HashMap::with_capacity(prewarmed.rasters.len() + 256),
            cursor: (PAD, PAD),
            row_h: 0,
            rasterised: 0,
        };
        // Pack everything into a CPU copy, then upload the used rows once.
        let mut pixels = vec![0u8; (ATLAS_W * ATLAS_H) as usize];
        for raster in &prewarmed.rasters {
            let Some((x, y)) = text.allocate(raster.metrics.width as i32, raster.metrics.height as i32) else {
                break;
            };
            for row in 0..raster.metrics.height {
                let src = &raster.bitmap[row * raster.metrics.width..][..raster.metrics.width];
                let dst = ((y as usize + row) * ATLAS_W as usize) + x as usize;
                pixels[dst..dst + raster.metrics.width].copy_from_slice(src);
            }
            text.insert(raster.key, &raster.metrics, x, y);
        }
        let used = (text.cursor.1 + text.row_h + PAD).min(ATLAS_H);
        renderer.upload(text.atlas, 0, 0, ATLAS_W, used, gl::ALPHA, &pixels[..(ATLAS_W * used) as usize]);
        text
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
