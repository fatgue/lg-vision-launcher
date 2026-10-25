//! Bitmap text at exact pixel sizes from a shared A8 glyph atlas. Glyphs rasterise once on
//! first use (fontdue) and upload as a tiny sub-image; steady-state frames do no font work.
use crate::color::Rgba;
use crate::gl;
use crate::render::{Rect, Renderer, Texture};
use std::collections::HashMap;
use std::path::Path;

const ATLAS_W: i32 = 2048;
const ATLAS_H: i32 = 1024;
const PAD: i32 = 1;

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

#[derive(Clone, Copy)]
struct Glyph {
    uv: [f32; 4],
    w: f32,
    h: f32,
    xmin: f32,
    ymin: f32,
    advance: f32,
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
    pub fn new(renderer: &Renderer, fonts_dir: &Path) -> Result<Self, String> {
        let load = |file: &str| -> Result<fontdue::Font, String> {
            let bytes = std::fs::read(fonts_dir.join(file)).map_err(|e| format!("{file}: {e}"))?;
            fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).map_err(|e| format!("{file}: {e}"))
        };
        Ok(Self {
            fonts: vec![load("Outfit-Regular.ttf")?, load("Outfit-Medium.ttf")?, load("Outfit-SemiBold.ttf")?],
            atlas: renderer.create_texture(ATLAS_W, ATLAS_H, gl::ALPHA),
            cache: HashMap::with_capacity(512),
            cursor: (PAD, PAD),
            row_h: 0,
            rasterised: 0,
        })
    }

    fn glyph(&mut self, renderer: &Renderer, key: Key) -> Glyph {
        if let Some(glyph) = self.cache.get(&key) {
            return *glyph;
        }
        let font = &self.fonts[key.weight as usize];
        let (metrics, bitmap) = font.rasterize(key.ch, key.size as f32);
        let (w, h) = (metrics.width as i32, metrics.height as i32);
        if self.cursor.0 + w + PAD > ATLAS_W {
            self.cursor = (PAD, self.cursor.1 + self.row_h + PAD);
            self.row_h = 0;
        }
        if self.cursor.1 + h + PAD > ATLAS_H {
            // Full: start over. Glyphs re-rasterise lazily; queued quads this frame may
            // briefly show a wrong glyph, which the next frame corrects.
            self.cache.clear();
            self.cursor = (PAD, PAD);
            self.row_h = 0;
        }
        let (x, y) = self.cursor;
        if w > 0 && h > 0 {
            renderer.upload(self.atlas, x, y, w, h, gl::ALPHA, &bitmap);
        }
        self.cursor.0 += w + PAD;
        self.row_h = self.row_h.max(h);
        self.rasterised += 1;
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

    fn key(style: Style, ch: char) -> Key {
        Key { weight: style.weight, size: style.size.round() as u16, ch }
    }

    pub fn measure(&mut self, renderer: &Renderer, text: &str, style: Style) -> f32 {
        let mut width = 0.0;
        let mut previous = None;
        let size = style.size.round();
        for ch in text.chars() {
            if let Some(prev) = previous {
                width += self.fonts[style.weight as usize].horizontal_kern(prev, ch, size).unwrap_or(0.0);
            }
            width += self.glyph(renderer, Self::key(style, ch)).advance + style.tracking * size;
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
            let glyph = self.glyph(renderer, Self::key(style, ch));
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
