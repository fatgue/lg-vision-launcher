//! Home screen state, input and drawing (direction A).
use crate::anim::Spring;
use crate::color::Rgba;
use crate::gl;
use crate::model::{App, Icon, Input};
use crate::render::{Rect, Renderer, Texture, LOGICAL_H, LOGICAL_W};
use crate::text::{Style, Text};
use crate::theme::*;
use serde_json::Value;
use std::collections::HashMap;
use std::time::{Duration, Instant};

const ICON_CELL: i32 = 128;
const ICON_ATLAS: i32 = 1024;
const RAIL_TITLE_Y: f32 = 620.0;
const RAIL_PITCH: f32 = 340.0;
const AMBIENT_SPEED: f32 = 6.0;

pub enum Action {
    Launch { app_id: String, params: Option<Value> },
    Exit,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Ok,
    Back,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    App,
    Input { connected: bool },
}

struct Tile {
    id: String,
    title: String,
    launch_id: String,
    params: Option<Value>,
    kind: Kind,
    bg: Rgba,
    accent: Rgba,
    icon: Option<[f32; 5]>,
    focus: Spring,
}

struct Rail {
    title: &'static str,
    tiles: Vec<Tile>,
    focus: usize,
    scroll: Spring,
    tile_w: f32,
    tile_h: f32,
}

impl Rail {
    fn new(title: &'static str, tile_w: f32, tile_h: f32) -> Self {
        Self { title, tiles: Vec::new(), focus: 0, scroll: Spring::new(0.0, 16.0), tile_w, tile_h }
    }

    fn tile_x(&self, index: usize) -> f32 {
        SAFE_X + index as f32 * (self.tile_w + TILE_GAP)
    }

    /// Minimal scroll that keeps the focused tile (with its focus growth) inside the safe area.
    fn follow_focus(&mut self) {
        let x = self.tile_x(self.focus);
        let left = x - self.scroll.target;
        let right = left + self.tile_w;
        if left < SAFE_X {
            self.scroll.target = x - SAFE_X;
        } else if right > LOGICAL_W - SAFE_X {
            self.scroll.target = x + self.tile_w - (LOGICAL_W - SAFE_X);
        }
        self.scroll.target = self.scroll.target.max(0.0);
    }
}

struct Hero {
    eyebrow: String,
    title: String,
    sub: String,
}

pub struct Home {
    rails: [Rail; 2],
    active: usize,
    vscroll: Spring,
    ambient: Rgba,
    hero: Option<Hero>,
    previous_hero: Option<Hero>,
    hero_t: Spring,
    icons: HashMap<String, [f32; 5]>,
    icon_atlas: Texture,
    icon_next: i32,
    pending_icons: HashMap<String, Icon>,
    notice: Option<(String, Instant)>,
    pub loaded: bool,
}

impl Home {
    pub fn new(renderer: &Renderer) -> Self {
        Self {
            rails: [Rail::new("Favoriten", 300.0, 170.0), Rail::new("Eingänge", 300.0, 120.0)],
            active: 0,
            vscroll: Spring::new(0.0, 14.0),
            ambient: ACCENT_DEFAULT,
            hero: None,
            previous_hero: None,
            hero_t: Spring::new(1.0, 14.0),
            icons: HashMap::new(),
            icon_atlas: renderer.create_texture(ICON_ATLAS, ICON_ATLAS, gl::RGBA),
            icon_next: 0,
            pending_icons: HashMap::new(),
            notice: None,
            loaded: false,
        }
    }

    pub fn set_apps(&mut self, apps: Vec<App>) {
        let previous = self.rails[0].tiles.get(self.rails[0].focus).map(|t| t.id.clone());
        self.rails[0].tiles = apps
            .into_iter()
            .map(|app| {
                let declared = app.bg_color.as_deref().and_then(Rgba::parse);
                let mut tile = Tile {
                    icon: self.icons.get(&app.id).copied(),
                    id: app.id.clone(),
                    title: app.title,
                    launch_id: app.id,
                    params: app.params,
                    kind: Kind::App,
                    bg: TILE_NEUTRAL,
                    accent: ACCENT_DEFAULT,
                    focus: Spring::new(0.0, 18.0),
                };
                if let Some(color) = declared {
                    tint(&mut tile, color);
                }
                tile
            })
            .collect();
        let rail = &mut self.rails[0];
        rail.focus = previous
            .and_then(|id| rail.tiles.iter().position(|t| t.id == id))
            .unwrap_or(0)
            .min(rail.tiles.len().saturating_sub(1));
        self.loaded = true;
        let pending: Vec<Icon> = self.pending_icons.drain().map(|(_, icon)| icon).collect();
        for icon in pending {
            self.add_icon(icon, None);
        }
        self.refocus(true);
    }

    pub fn set_inputs(&mut self, inputs: Vec<Input>) {
        self.rails[1].tiles = inputs
            .into_iter()
            .map(|input| Tile {
                id: input.id,
                title: input.label,
                launch_id: input.app_id,
                params: None,
                kind: Kind::Input { connected: input.connected },
                bg: Rgba::hex(0x14161C),
                accent: ACCENT_INPUT,
                icon: None,
                focus: Spring::new(0.0, 18.0),
            })
            .collect();
        let rail = &mut self.rails[1];
        rail.focus = rail.focus.min(rail.tiles.len().saturating_sub(1));
        self.refocus(true);
    }

    /// Packs an icon into the shared atlas (one upload, never re-sent) and tints its tile.
    pub fn add_icon(&mut self, icon: Icon, renderer: Option<&Renderer>) {
        let Some(tile) = self.rails[0].tiles.iter_mut().find(|t| t.id == icon.app_id) else {
            self.pending_icons.insert(icon.app_id.clone(), icon);
            return;
        };
        if let Some(color) = icon.color {
            tint(tile, Rgba::from_u8(color));
        }
        if let Some(slot) = self.icons.get(&icon.app_id) {
            tile.icon = Some(*slot);
            return;
        }
        let Some(renderer) = renderer else {
            self.pending_icons.insert(icon.app_id.clone(), icon);
            return;
        };
        let per_row = ICON_ATLAS / ICON_CELL;
        if self.icon_next >= per_row * per_row {
            return;
        }
        let (cx, cy) = ((self.icon_next % per_row) * ICON_CELL, (self.icon_next / per_row) * ICON_CELL);
        self.icon_next += 1;
        renderer.upload(self.icon_atlas, cx, cy, icon.w as i32, icon.h as i32, gl::RGBA, &icon.pixels);
        let a = ICON_ATLAS as f32;
        let slot = [
            cx as f32 / a,
            cy as f32 / a,
            (cx + icon.w as i32) as f32 / a,
            (cy + icon.h as i32) as f32 / a,
            icon.w as f32 / icon.h as f32,
        ];
        tile.icon = Some(slot);
        self.icons.insert(icon.app_id, slot);
    }

    /// Icons that arrived before a renderer pass are uploaded here.
    pub fn flush_pending_icons(&mut self, renderer: &Renderer) {
        if self.pending_icons.is_empty() || self.rails[0].tiles.is_empty() {
            return;
        }
        let ready: Vec<String> = self
            .pending_icons
            .keys()
            .filter(|id| self.rails[0].tiles.iter().any(|t| &t.id == *id))
            .cloned()
            .collect();
        for id in ready {
            if let Some(icon) = self.pending_icons.remove(&id) {
                self.add_icon(icon, Some(renderer));
            }
        }
        self.refocus(false);
    }

    pub fn notice(&mut self, text: String) {
        self.notice = Some((text, Instant::now()));
    }

    pub fn key(&mut self, key: Key, home_mode: bool) -> Option<Action> {
        let active = self.active;
        let (focus, count) = (self.rails[active].focus, self.rails[active].tiles.len());
        let has = |rail: usize| self.rails.get(rail).is_some_and(|r| !r.tiles.is_empty());
        match key {
            Key::Left if focus > 0 => self.rails[active].focus -= 1,
            Key::Right if focus + 1 < count => self.rails[active].focus += 1,
            Key::Up if active > 0 && has(active - 1) => self.active -= 1,
            Key::Down if has(active + 1) => self.active += 1,
            Key::Ok => {
                let tile = self.focused()?;
                return Some(Action::Launch { app_id: tile.launch_id.clone(), params: tile.params.clone() });
            }
            Key::Back if focus > 0 => self.rails[active].focus = 0,
            Key::Back if active > 0 => self.active = 0,
            Key::Back if !home_mode => return Some(Action::Exit),
            _ => return None,
        }
        self.refocus(false);
        None
    }

    fn focused(&self) -> Option<&Tile> {
        let rail = &self.rails[self.active];
        rail.tiles.get(rail.focus)
    }

    fn refocus(&mut self, instant: bool) {
        for (r, rail) in self.rails.iter_mut().enumerate() {
            for (i, tile) in rail.tiles.iter_mut().enumerate() {
                tile.focus.target = if r == self.active && i == rail.focus { 1.0 } else { 0.0 };
                if instant {
                    tile.focus.snap();
                }
            }
            if !rail.tiles.is_empty() {
                rail.follow_focus();
            }
            if instant {
                rail.scroll.snap();
            }
        }
        self.vscroll.target = -(self.active as f32) * RAIL_PITCH;
        let next = self.focused().map(|tile| Hero {
            eyebrow: self.rails[self.active].title.to_uppercase(),
            title: tile.title.clone(),
            sub: match tile.kind {
                Kind::App => "OK drücken zum Öffnen".to_string(),
                Kind::Input { connected: true } => "Verbunden · OK zum Umschalten".to_string(),
                Kind::Input { connected: false } => "Kein Signal".to_string(),
            },
        });
        let changed = match (&self.hero, &next) {
            (Some(a), Some(b)) => a.title != b.title || a.eyebrow != b.eyebrow,
            (None, None) => false,
            _ => true,
        };
        if changed {
            self.previous_hero = self.hero.take();
            self.hero = next;
            self.hero_t = Spring::new(if instant { 1.0 } else { 0.0 }, 14.0);
            self.hero_t.target = 1.0;
        }
        if instant {
            vsnap(&mut self.vscroll);
            if let Some(tile) = self.focused() {
                self.ambient = tile.accent;
            }
        }
    }

    /// Advances animations; returns true while anything still moves.
    pub fn update(&mut self, dt: f32) -> bool {
        let mut moving = false;
        for rail in &mut self.rails {
            moving |= rail.scroll.step(dt);
            for tile in &mut rail.tiles {
                moving |= tile.focus.step(dt);
            }
        }
        moving |= self.vscroll.step(dt);
        moving |= self.hero_t.step(dt);
        let target = self.focused().map_or(ACCENT_DEFAULT, |t| t.accent);
        if self.ambient.distance(target) > 0.004 {
            self.ambient = self.ambient.lerp(target, 1.0 - (-AMBIENT_SPEED * dt).exp());
            moving = true;
        } else {
            self.ambient = target;
        }
        if let Some((_, since)) = &self.notice {
            if since.elapsed() > Duration::from_secs(4) {
                self.notice = None;
            } else {
                moving = true;
            }
        }
        moving
    }

    pub fn settle(&mut self) {
        for rail in &mut self.rails {
            rail.scroll.snap();
            for tile in &mut rail.tiles {
                tile.focus.snap();
            }
        }
        vsnap(&mut self.vscroll);
        self.hero_t.snap();
        self.previous_hero = None;
    }

    pub fn draw(&mut self, r: &mut Renderer, text: &mut Text, clock: &str, date: &str) {
        self.draw_backdrop(r);
        self.draw_header(r, text, clock, date);
        self.draw_hero(r, text);
        self.draw_rails(r, text);
        self.draw_notice(r, text);
    }

    fn draw_backdrop(&self, r: &mut Renderer) {
        // Ambient light from the focused tile's colour, upper right.
        r.radial(1430.0, 280.0, 750.0, 600.0, self.ambient.alpha(0.55));
        // Bottom vignette keeps rails legible over the light.
        r.vgradient(Rect::new(0.0, 460.0, LOGICAL_W, 248.0), BG.alpha(0.0), BG.alpha(0.85));
        r.vgradient(Rect::new(0.0, 708.0, LOGICAL_W, 372.0), BG.alpha(0.85), BG);
    }

    fn draw_header(&self, r: &mut Renderer, text: &mut Text, clock: &str, date: &str) {
        let (cx, cy) = (SAFE_X + 17.0, SAFE_TOP + 24.0);
        r.stroke(Rect::new(cx - 15.0, cy - 15.0, 30.0, 30.0), 15.0, 2.5, TEXT);
        r.rect(Rect::new(cx - 6.0, cy - 6.0, 12.0, 12.0), 6.0, self.ambient);
        text.draw_top(r, "Vision", SAFE_X + 48.0, SAFE_TOP + 7.0, BRAND, TEXT);

        let clock_w = text.measure(r, clock, CLOCK);
        let clock_x = LOGICAL_W - SAFE_X - clock_w;
        text.draw(r, clock, clock_x, SAFE_TOP + 39.0, CLOCK, TEXT);
        let date_w = text.measure(r, date, HEADER);
        text.draw(r, date, clock_x - 40.0 - date_w, SAFE_TOP + 37.0, HEADER, TEXT.alpha(0.78));
    }

    fn draw_hero(&self, r: &mut Renderer, text: &mut Text) {
        let t = self.hero_t.value.clamp(0.0, 1.0);
        if let Some(hero) = &self.previous_hero {
            if t < 1.0 {
                draw_hero_block(r, text, hero, 1.0 - t, -10.0 * t, self.ambient);
            }
        }
        if let Some(hero) = &self.hero {
            draw_hero_block(r, text, hero, t, 14.0 * (1.0 - t), self.ambient);
        }
    }

    fn draw_rails(&self, r: &mut Renderer, text: &mut Text) {
        // Three passes keep texture switches (shapes → icon atlas → glyph atlas) to a minimum.
        for pass in 0..3 {
            for (index, rail) in self.rails.iter().enumerate() {
                if rail.tiles.is_empty() {
                    continue;
                }
                let title_y = RAIL_TITLE_Y + index as f32 * RAIL_PITCH + self.vscroll.value;
                let alpha = ((title_y - 380.0) / 240.0).clamp(0.0, 1.0);
                if alpha <= 0.0 || title_y > LOGICAL_H {
                    continue;
                }
                let tiles_y = title_y + 66.0;
                if pass == 2 {
                    let dim = if index == self.active { 0.7 } else { 0.5 };
                    text.draw_top(r, rail.title, SAFE_X, title_y, RAIL_TITLE, TEXT.alpha(dim * alpha));
                }
                for (i, tile) in rail.tiles.iter().enumerate() {
                    let base = Rect::new(rail.tile_x(i) - rail.scroll.value, tiles_y, rail.tile_w, rail.tile_h);
                    if base.x > LOGICAL_W || base.x + base.w < -40.0 {
                        continue;
                    }
                    let f = tile.focus.value.clamp(0.0, 1.0);
                    let rect = base.scale_about(1.0 + (FOCUS_SCALE - 1.0) * f, 0.5, 1.0);
                    match pass {
                        0 => draw_tile_shapes(r, tile, rect, f, alpha),
                        1 => draw_tile_icon(r, self.icon_atlas, tile, rect, alpha),
                        _ => draw_tile_text(r, text, tile, rect, f, alpha),
                    }
                }
            }
        }
    }

    fn draw_notice(&self, r: &mut Renderer, text: &mut Text) {
        let Some((message, since)) = &self.notice else { return };
        let age = since.elapsed().as_secs_f32();
        let alpha = (age * 4.0).min(1.0).min((4.0 - age) * 2.0).clamp(0.0, 1.0);
        let w = text.measure(r, message, NOTICE) + 56.0;
        let rect = Rect::new(LOGICAL_W - SAFE_X - w, LOGICAL_H - 54.0 - 64.0, w, 64.0);
        r.rect(rect, 32.0, Rgba::hex(0x14161C).alpha(0.92 * alpha));
        r.stroke(rect, 32.0, 1.0, TEXT.alpha(0.14 * alpha));
        text.draw(r, message, rect.x + 28.0, rect.y + 40.0, NOTICE, TEXT.alpha(0.85 * alpha));
    }
}

fn vsnap(spring: &mut Spring) {
    spring.snap();
}

fn tint(tile: &mut Tile, color: Rgba) {
    tile.bg = BG.lerp(color, 0.2);
    tile.accent = color.luminous(0.92);
}

fn draw_hero_block(r: &mut Renderer, text: &mut Text, hero: &Hero, alpha: f32, dy: f32, accent: Rgba) {
    if alpha <= 0.01 {
        return;
    }
    let x = SAFE_X;
    let mut y = 210.0 + dy;
    text.draw_top(r, &hero.eyebrow, x, y, EYEBROW, accent.alpha(alpha));
    y += 46.0;
    let style = HERO_SIZES
        .iter()
        .map(|&size| crate::theme::hero(size))
        .find(|&style| text.measure(r, &hero.title, style) <= 1180.0)
        .unwrap_or(crate::theme::hero(HERO_SIZES[HERO_SIZES.len() - 1]));
    text.draw_top(r, &hero.title, x - 4.0, y, style, TEXT.alpha(alpha));
    y += style.size * 1.08 + 18.0;
    text.draw_top(r, &hero.sub, x, y, HERO_SUB, TEXT.alpha(0.74 * alpha));
}

fn draw_tile_shapes(r: &mut Renderer, tile: &Tile, rect: Rect, f: f32, alpha: f32) {
    if f > 0.0 {
        r.shadow(rect, TILE_RADIUS * 2.0, 80.0, tile.accent.alpha(0.45 * f * alpha));
        r.shadow(rect.offset(0.0, 30.0), TILE_RADIUS, 60.0, Rgba::hex(0).alpha(0.55 * f * alpha));
    }
    r.rect(rect, TILE_RADIUS, tile.bg.fade(alpha));
    if f < 1.0 {
        r.stroke(rect.inflate(-0.5), TILE_RADIUS, 1.0, TEXT.alpha(0.10 * (1.0 - f) * alpha));
    }
    if let Kind::Input { connected } = tile.kind {
        let dot = if connected { CONNECTED } else { TEXT.alpha(0.3) };
        r.rect(Rect::new(rect.x + 26.0, rect.y + rect.h * 0.5 - 6.0, 12.0, 12.0), 6.0, dot.fade(alpha));
    }
}

fn draw_tile_icon(r: &mut Renderer, atlas: Texture, tile: &Tile, rect: Rect, alpha: f32) {
    let Some([u0, v0, u1, v1, aspect]) = tile.icon else { return };
    let side = rect.h * 0.58;
    let (w, h) = if aspect >= 1.0 { (side * aspect.min(1.8), side) } else { (side, side / aspect) };
    let icon = Rect::new(rect.x + (rect.w - w) * 0.5, rect.y + (rect.h - h) * 0.5, w, h);
    r.image(atlas, icon, [u0, v0, u1, v1], 18.0 * rect.h / 170.0, Rgba::hex(0xFFFFFF).alpha(alpha));
}

fn draw_tile_text(r: &mut Renderer, text: &mut Text, tile: &Tile, rect: Rect, f: f32, alpha: f32) {
    if f > 0.0 {
        r.stroke(rect.inflate(RING_OFFSET + RING_WIDTH * 0.5), TILE_RADIUS + RING_OFFSET, RING_WIDTH, TEXT.alpha(f * alpha));
    }
    match tile.kind {
        Kind::App => {
            if tile.icon.is_none() {
                let style = fit(text, r, &tile.title, MONOGRAM, rect.w - 40.0);
                let w = text.measure(r, &tile.title, style);
                text.draw(r, &tile.title, rect.x + (rect.w - w) * 0.5, rect.y + rect.h * 0.5 + style.size * 0.35, style, TEXT.alpha(alpha));
            }
            if f > 0.01 {
                let w = text.measure(r, &tile.title, TILE_LABEL);
                let x = rect.x + (rect.w - w) * 0.5;
                text.draw_top(r, &tile.title, x, rect.y + rect.h + 22.0, TILE_LABEL, TEXT.alpha(f * alpha));
            }
        }
        Kind::Input { connected } => {
            let x = rect.x + 52.0;
            text.draw(r, &tile.title, x, rect.y + rect.h * 0.5 - 2.0, INPUT_LABEL, TEXT.alpha(alpha));
            let sub = if connected { "Verbunden" } else { "Kein Signal" };
            text.draw(r, sub, x, rect.y + rect.h * 0.5 + 26.0, INPUT_SUB, TEXT.alpha(0.6 * alpha));
        }
    }
}

fn fit(text: &mut Text, r: &Renderer, label: &str, style: Style, width: f32) -> Style {
    let mut style = style;
    while style.size > 20.0 && text.measure(r, label, style) > width {
        style.size -= 4.0;
    }
    style
}
