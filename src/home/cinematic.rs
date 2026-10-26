//! Drawing of the home screen in the Cinematic look.
use super::*;

impl Home {
    pub fn draw(&mut self, r: &mut Renderer, text: &mut Text, now: &chrono::DateTime<chrono::Local>) {
        match self.look {
            Look::Cinematic => self.draw_cinematic(r, text, now),
            Look::Glass => self.draw_glass(r, text, now),
        }
    }

    /// The header's clock and (when shown) date.
    pub(super) fn clock_and_date(&self, now: &chrono::DateTime<chrono::Local>) -> (String, String) {
        use chrono::{Datelike, Timelike};
        let ui = &self.prefs.ui;
        let clock = if ui.clock24 {
            format!("{:02}:{:02}", now.hour(), now.minute())
        } else {
            let (pm, hour) = now.hour12();
            format!("{hour}:{:02} {}", now.minute(), if pm { "PM" } else { "AM" })
        };
        let date = if ui.show_date {
            self.strings.date(now.weekday().num_days_from_monday() as usize, now.day(), now.month0() as usize)
        } else {
            String::new()
        };
        (clock, date)
    }

    pub(super) fn draw_cinematic(&mut self, r: &mut Renderer, text: &mut Text, now: &chrono::DateTime<chrono::Local>) {
        self.hits.borrow_mut().clear();
        let (clock, date) = self.clock_and_date(now);
        self.draw_backdrop(r);
        if self.preview.is_some() {
            r.punch(PREVIEW_RECT);
            r.stroke(PREVIEW_RECT.inflate(1.0), 4.0, 1.5, TEXT.alpha(0.25));
            let pill = Rect::new(PREVIEW_RECT.x + 20.0, PREVIEW_RECT.y + 20.0, 84.0, 36.0);
            r.rect(pill, 8.0, Rgba::hex(0xE5484D));
            text.draw(r, &self.strings.live.to_uppercase(), pill.x + 14.0, pill.y + 26.0, INPUT_SUB, Rgba::hex(0xFFFFFF));
        }
        self.draw_header(r, text, &clock, &date);
        self.draw_hero(r, text);
        self.draw_rails(r, text);
        self.draw_menu(r, text);
        self.draw_setup(r, text);
        self.draw_offer(r, text);
        self.draw_settings(r, text);
        self.draw_store(r, text);
        self.draw_homebrew(r, text);
        self.draw_picker(r, text);
        self.draw_digits(r, text);
        self.draw_notice(r, text);
    }

    pub(super) fn draw_backdrop(&self, r: &mut Renderer) {
        // Ambient light from the focused tile's colour, upper right.
        let strength = [0.0, 0.3, 0.55, 0.8][self.prefs.ui.ambient.min(3) as usize];
        r.radial(1430.0, 280.0, 750.0, 600.0, self.ambient.alpha(strength));
        // Bottom vignette keeps rails legible over the light.
        r.vgradient(Rect::new(0.0, 460.0, LOGICAL_W, 248.0), BG.alpha(0.0), BG.alpha(0.85));
        r.vgradient(Rect::new(0.0, 708.0, LOGICAL_W, 372.0), BG.alpha(0.85), BG);
    }

    pub(super) fn draw_header(&self, r: &mut Renderer, text: &mut Text, clock: &str, date: &str) {
        let (cx, cy) = (SAFE_X + 17.0, SAFE_TOP + 24.0);
        r.stroke(Rect::new(cx - 15.0, cy - 15.0, 30.0, 30.0), 15.0, 2.5, TEXT);
        r.rect(Rect::new(cx - 6.0, cy - 6.0, 12.0, 12.0), 6.0, self.ambient);
        text.draw_top(r, "Vision", SAFE_X + 48.0, SAFE_TOP + 7.0, BRAND, TEXT);

        // Settings gear, far right; focusable from the top row with Up.
        let gear = Rect::new(LOGICAL_W - SAFE_X - 48.0, SAFE_TOP, 48.0, 48.0);
        self.hits.borrow_mut().push((gear.inflate(8.0), Target::Gear));
        draw_gear(r, gear, if self.header { TEXT } else { TEXT.alpha(0.7) });
        if self.header {
            r.stroke(gear.inflate(10.0), 34.0, 3.0, TEXT);
        }
        let clock_w = text.measure(r, clock, CLOCK);
        let mut x = gear.x - 36.0 - clock_w;
        text.draw(r, clock, x, SAFE_TOP + 39.0, CLOCK, TEXT);
        if !date.is_empty() {
            x -= 40.0 + text.measure(r, date, HEADER);
            text.draw(r, date, x, SAFE_TOP + 37.0, HEADER, TEXT.alpha(0.78));
        }
        if let Some(weather) = self.weather.filter(|_| self.prefs.ui.weather) {
            let temperature = if self.prefs.ui.fahrenheit { weather.temperature * 9.0 / 5.0 + 32.0 } else { weather.temperature };
            let label = format!("{temperature:.0}° · {}", self.strings.weather(weather.code));
            x -= 40.0 + text.measure(r, &label, HEADER);
            text.draw(r, &label, x, SAFE_TOP + 37.0, HEADER, TEXT.alpha(0.78));
        }
    }

    pub(super) fn draw_hero(&self, r: &mut Renderer, text: &mut Text) {
        let t = self.hero_t.value.clamp(0.0, 1.0);
        let accent = if self.lift.target > 0.0 { TEXT } else { self.ambient };
        // With a preview playing, the text keeps clear of the video.
        let width = if self.preview.is_some() { PREVIEW_RECT.x - SAFE_X - 40.0 } else { 1180.0 };
        if let Some(hero) = &self.previous_hero {
            if t < 1.0 {
                draw_hero_block(r, text, hero, 1.0 - t, -10.0 * t, accent, width);
            }
        }
        if let Some(hero) = &self.hero {
            draw_hero_block(r, text, hero, t, 14.0 * (1.0 - t), accent, width);
        }
    }

    pub(super) fn draw_rails(&self, r: &mut Renderer, text: &mut Text) {
        let lift = self.lift.value.clamp(0.0, 1.0);
        // Passes keep texture switches to a minimum: shapes → artwork → icons → glyphs.
        for pass in 0..4 {
            for (index, rail) in self.rails.iter().enumerate() {
                if !self.has(index) {
                    continue;
                }
                let title_y = RAIL_TITLE_Y + self.rail_slot(index) as f32 * RAIL_PITCH + self.vscroll.value;
                let alpha = ((title_y - 380.0) / 240.0).clamp(0.0, 1.0);
                if alpha <= 0.0 || title_y > LOGICAL_H {
                    continue;
                }
                let tiles_y = title_y + 66.0;
                if pass == 3 {
                    let dim = if index == self.active { 0.7 } else { 0.5 };
                    text.draw_top(r, self.rail_title(index), SAFE_X, title_y, RAIL_TITLE, TEXT.alpha(dim * alpha));
                }
                for (i, tile) in rail.tiles.iter().enumerate() {
                    let base = Rect::new(rail.slot_x(tile.slot.value) - rail.scroll.value, tiles_y, rail.tile_w, rail.tile_h);
                    if base.x > LOGICAL_W || base.x + base.w < -40.0 {
                        continue;
                    }
                    let f = tile.focus.value.clamp(0.0, 1.0);
                    let lifted = if index == self.active && i == rail.focus { lift } else { 0.0 };
                    let scale = 1.0 + (FOCUS_SCALE - 1.0) * f + 0.05 * lifted;
                    let rect = base.scale_about(scale, 0.5, 1.0).offset(0.0, -14.0 * lifted);
                    let view = TileView {
                        now: self.channel_now(tile),
                        rect,
                        f,
                        alpha,
                        lifted,
                        icon: self.icons.get(&tile.id).copied(),
                        art: self.arts.get(&tile.id).map(|(_, uv)| *uv),
                        fill: self.blended.get(&tile.id).copied(),
                        radius: TILE_RADIUS,
                        ring: true,
                    };
                    if pass == 0 && alpha > 0.5 && matches!(self.mode, Mode::Browse) {
                        self.hits.borrow_mut().push((rect, Target::Tile { rail: index, index: i }));
                    }
                    match pass {
                        0 => draw_tile_shapes(r, tile, &view),
                        1 => draw_tile_art(r, self.art_atlas, &view),
                        2 => draw_tile_icon(r, self.icon_atlas, &view),
                        _ => draw_tile_text(r, text, tile, &view, self.strings),
                    }
                }
            }
        }
    }

    pub(super) fn channel_now(&self, tile: &Tile) -> Option<(String, f32)> {
        let Kind::Channel { id } = tile.kind else { return None };
        let now_ms = crate::clock::utc_ms();
        let (now, _) = self.tv.as_ref()?.now_next(id, now_ms);
        let now = now?;
        let progress = (now_ms - now.start) as f32 / (now.end - now.start).max(1) as f32;
        Some((now.title.clone(), progress.clamp(0.0, 1.0)))
    }
}

pub(super) struct TileView {
    /// Guide position for channel tiles: (programme title, progress).
    pub(super) now: Option<(String, f32)>,
    pub(super) rect: Rect,
    pub(super) f: f32,
    pub(super) alpha: f32,
    pub(super) lifted: f32,
    pub(super) icon: Option<[f32; 5]>,
    pub(super) art: Option<[f32; 4]>,
    pub(super) fill: Option<Rgba>,
    pub(super) radius: f32,
    /// Cinematic focus: coloured glow, ring and the title under the tile. Glass draws its own.
    pub(super) ring: bool,
}

pub(super) fn draw_hero_block(r: &mut Renderer, text: &mut Text, hero: &Hero, alpha: f32, dy: f32, accent: Rgba, width: f32) {
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
        .find(|&style| text.measure(r, &hero.title, style) <= width)
        .unwrap_or(crate::theme::hero(HERO_SIZES[HERO_SIZES.len() - 1]));
    let title = clip(text, r, &hero.title, style, width);
    text.draw_top(r, &title, x - 4.0, y, style, TEXT.alpha(alpha));
    y += style.size * 1.08 + 18.0;
    let sub = clip(text, r, &hero.sub, HERO_SUB, width);
    text.draw_top(r, &sub, x, y, HERO_SUB, TEXT.alpha(0.74 * alpha));
    if let Some(hint) = hero.hint {
        text.draw_top(r, hint, x, y + 50.0, HINT, TEXT.alpha(0.5 * alpha));
    }
}

pub(super) fn draw_tile_shapes(r: &mut Renderer, tile: &Tile, v: &TileView) {
    let (rect, f, alpha) = (v.rect, v.f, v.alpha);
    if v.ring && f > 0.0 {
        r.shadow(rect, TILE_RADIUS * 2.0, 80.0, tile.accent.alpha(0.45 * f * alpha));
        r.shadow(rect.offset(0.0, 30.0 + 16.0 * v.lifted), TILE_RADIUS, 60.0 + 30.0 * v.lifted, Rgba::hex(0).alpha(0.55 * f * alpha));
    }
    let fill = match (v.art, v.fill) {
        (None, Some(edge)) => edge,
        _ => tile.bg,
    };
    r.rect(rect, v.radius, fill.fade(alpha));
    match tile.kind {
        Kind::Input { connected } | Kind::Setting { connected, .. } => {
            let dot = if connected { CONNECTED } else { TEXT.alpha(0.3) };
            r.rect(Rect::new(rect.x + 26.0, rect.y + rect.h * 0.5 - 6.0, 12.0, 12.0), 6.0, dot.fade(alpha));
        }
        Kind::Promo => {
            // TiviOne's teal, brighter towards the bottom, with its wordmark colours as a bar.
            r.vgradient(rect, tile.bg.fade(alpha), Rgba::hex(0x0F5A5A).fade(alpha));
            r.rect(Rect::new(rect.x + 24.0, rect.y + rect.h - 30.0, 56.0, 6.0), 3.0, TIVIONE_TEAL.fade(alpha));
        }
        Kind::Entity { on } => {
            let lamp = match on {
                Some(true) => Rgba::hex(0xF2C14E),
                Some(false) => TEXT.alpha(0.25),
                None => tile.accent.alpha(0.8),
            };
            r.rect(Rect::new(rect.x + 24.0, rect.y + 24.0, 16.0, 16.0), 8.0, lamp.fade(alpha));
        }
        _ => {}
    }
}

pub(super) fn draw_tile_art(r: &mut Renderer, atlas: Texture, v: &TileView) {
    let Some(uv) = v.art else { return };
    // Unfocused artwork sits back slightly so the focused tile reads first.
    let light = 0.78 + 0.22 * v.f;
    r.image(atlas, v.rect, uv, v.radius, Rgba { r: light, g: light, b: light, a: v.alpha });
}

pub(super) fn draw_tile_icon(r: &mut Renderer, atlas: Texture, v: &TileView) {
    if v.art.is_some() {
        return;
    }
    let Some([u0, v0, u1, v1, aspect]) = v.icon else { return };
    let rect = v.rect;
    // An icon whose background fills the tile is shown large: the tile becomes the logo.
    let side = rect.h * if v.fill.is_some() { 0.86 } else { 0.58 };
    let (w, h) = if aspect >= 1.0 { (side * aspect.min(1.8), side) } else { (side, side / aspect) };
    let icon = Rect::new(rect.x + (rect.w - w) * 0.5, rect.y + (rect.h - h) * 0.5, w, h);
    let radius = if v.fill.is_some() { 0.0 } else { 18.0 * rect.h / 170.0 };
    r.image(atlas, icon, [u0, v0, u1, v1], radius, Rgba::hex(0xFFFFFF).alpha(v.alpha));
}

pub(super) fn draw_tile_text(r: &mut Renderer, text: &mut Text, tile: &Tile, v: &TileView, s: &Strings) {
    let (rect, f, alpha) = (v.rect, v.f, v.alpha);
    if v.ring && f < 1.0 {
        r.stroke(rect.inflate(-0.5), TILE_RADIUS, 1.0, TEXT.alpha(0.10 * (1.0 - f) * alpha));
    }
    if v.ring && f > 0.0 {
        let ring = TEXT.lerp(tile.accent, v.lifted);
        r.stroke(rect.inflate(RING_OFFSET + RING_WIDTH * 0.5), TILE_RADIUS + RING_OFFSET, RING_WIDTH, ring.alpha(f * alpha));
    }
    let label_below = |text: &mut Text, r: &mut Renderer| {
        if v.ring && f > 0.01 {
            let w = text.measure(r, &tile.title, TILE_LABEL);
            text.draw_top(r, &tile.title, rect.x + (rect.w - w) * 0.5, rect.y + rect.h + 22.0, TILE_LABEL, TEXT.alpha(f * alpha));
        }
    };
    match tile.kind {
        Kind::App => {
            if v.icon.is_none() && v.art.is_none() {
                let style = fit(text, r, &tile.title, MONOGRAM, rect.w - 40.0);
                let w = text.measure(r, &tile.title, style);
                text.draw(r, &tile.title, rect.x + (rect.w - w) * 0.5, rect.y + rect.h * 0.5 + style.size * 0.35, style, TEXT.alpha(alpha));
            }
            label_below(text, r);
        }
        Kind::Media { progress } => {
            if v.art.is_none() {
                let style = fit(text, r, &tile.title, MONOGRAM, rect.w - 40.0);
                let w = text.measure(r, &tile.title, style);
                text.draw(r, &tile.title, rect.x + (rect.w - w) * 0.5, rect.y + rect.h * 0.5 + style.size * 0.35, style, TEXT.alpha(alpha));
            }
            let track = Rect::new(rect.x + 18.0, rect.y + rect.h - 18.0, rect.w - 36.0, 5.0);
            r.rect(track, 2.5, Rgba::hex(0x000000).alpha(0.45 * alpha));
            r.rect(Rect::new(track.x, track.y, track.w * progress, track.h), 2.5, TEXT.alpha(0.95 * alpha));
            label_below(text, r);
        }
        Kind::Entity { .. } => {
            let style = fit(text, r, &tile.title, ENTITY_NAME, rect.w - 48.0);
            text.draw(r, &tile.title, rect.x + 24.0, rect.y + rect.h - 52.0, style, TEXT.alpha(alpha));
            text.draw(r, &tile.detail, rect.x + 24.0, rect.y + rect.h - 22.0, INPUT_SUB, TEXT.alpha(0.6 * alpha));
        }
        Kind::Input { connected } => {
            let x = rect.x + 52.0;
            text.draw(r, &tile.title, x, rect.y + rect.h * 0.5 - 2.0, INPUT_LABEL, TEXT.alpha(alpha));
            let sub = if connected { s.connected } else { s.no_signal };
            text.draw(r, sub, x, rect.y + rect.h * 0.5 + 26.0, INPUT_SUB, TEXT.alpha(0.6 * alpha));
        }
        Kind::Channel { .. } => {
            let (x, w) = (rect.x + 24.0, rect.w - 48.0);
            let mut top = rect.y + 20.0;
            if !tile.detail.is_empty() {
                text.draw_top(r, &tile.detail, x, top, INPUT_SUB, TEXT.alpha(0.6 * alpha));
                top += 28.0;
            }
            let name = clip(text, r, tile.title.trim(), ENTITY_NAME, w);
            text.draw_top(r, &name, x, top, ENTITY_NAME, TEXT.alpha(alpha));
            if let Some((title, progress)) = &v.now {
                let line = clip(text, r, title, INPUT_LABEL, w);
                text.draw(r, &line, x, rect.y + rect.h - 40.0, INPUT_LABEL, TEXT.alpha(alpha));
                let track = Rect::new(x, rect.y + rect.h - 24.0, w, 4.0);
                r.rect(track, 2.0, TEXT.alpha(0.18 * alpha));
                r.rect(Rect::new(track.x, track.y, track.w * progress, 4.0), 2.0, TEXT.alpha(0.9 * alpha));
            }
        }
        Kind::Catchup { start, end, .. } => {
            let x = rect.x + 24.0;
            text.draw_top(r, &format!("{} – {}", clock_text(start), clock_text(end)), x, rect.y + 20.0, INPUT_SUB, TEXT.alpha(0.7 * alpha));
            let line = clip(text, r, &tile.title, INPUT_LABEL, rect.w - 48.0);
            text.draw(r, &line, x, rect.y + rect.h - 52.0, INPUT_LABEL, TEXT.alpha(alpha));
            text.draw(r, &tile.detail, x, rect.y + rect.h - 22.0, INPUT_SUB, TEXT.alpha(0.6 * alpha));
        }
        Kind::Reminder { start, .. } => {
            let x = rect.x + 24.0;
            text.draw_top(r, &clock_text(start), x, rect.y + 18.0, ENTITY_NAME, TEXT.alpha(alpha));
            let line = clip(text, r, &tile.title, INPUT_LABEL, rect.w - 48.0);
            text.draw(r, &line, x, rect.y + rect.h - 52.0, INPUT_LABEL, TEXT.alpha(alpha));
            text.draw(r, &tile.detail, x, rect.y + rect.h - 22.0, INPUT_SUB, TEXT.alpha(0.6 * alpha));
        }
        Kind::Promo => {
            let x = rect.x + 24.0;
            text.draw_top(r, &tile.title, x, rect.y + 22.0, MONOGRAM, TEXT.alpha(alpha));
            let detail = clip(text, r, &tile.detail, INPUT_SUB, rect.w - 48.0);
            text.draw(r, &detail, x, rect.y + rect.h - 44.0, INPUT_SUB, TEXT.alpha(0.75 * alpha));
        }
        Kind::Setting { .. } => {
            let x = rect.x + 52.0;
            let style = fit(text, r, &tile.title, INPUT_LABEL, rect.w - 70.0);
            text.draw(r, &tile.title, x, rect.y + rect.h * 0.5 - 2.0, style, TEXT.alpha(alpha));
            text.draw(r, &tile.detail, x, rect.y + rect.h * 0.5 + 26.0, INPUT_SUB, TEXT.alpha(0.6 * alpha));
        }
    }
}

/// A small gear: a ring with eight teeth and a hub.
pub(super) fn draw_gear(r: &mut Renderer, area: Rect, color: Rgba) {
    draw_gear_at(r, area.x + area.w * 0.5, area.y + area.h * 0.5, 1.0, color);
}

pub(super) fn draw_gear_at(r: &mut Renderer, cx: f32, cy: f32, scale: f32, color: Rgba) {
    for i in 0..8 {
        let angle = i as f32 * std::f32::consts::FRAC_PI_4;
        let (tx, ty) = (cx + angle.cos() * 15.0 * scale, cy + angle.sin() * 15.0 * scale);
        let tooth = 9.0 * scale;
        r.rect(Rect::new(tx - tooth * 0.5, ty - tooth * 0.5, tooth, tooth), 3.0 * scale, color);
    }
    let ring = 22.0 * scale;
    r.stroke(Rect::new(cx - ring * 0.5, cy - ring * 0.5, ring, ring), ring * 0.5, 6.0 * scale, color);
}
