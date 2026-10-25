//! The Glass look, inspired by tvOS: a Top Shelf showing what the focused app offers, a
//! five-column app grid with the Settings tile in its last slot, and rows (Home Assistant,
//! inputs, hidden apps) below. Focus lifts a tile without a ring; only its title shows.
use super::cinematic::*;
use super::*;

pub(super) const COLS: usize = 5;
pub(super) const TILE: (f32, f32) = (308.0, 185.0);
/// Five tiles and four gaps fill the 1728 px between the safe margins.
pub(super) const GAP: f32 = 47.0;
pub(super) const CARD: (f32, f32) = (300.0, 169.0);
pub(super) const CARD_GAP: f32 = 32.0;
pub(super) const ROW_GAP: f32 = 40.0;
const ROW_PITCH: f32 = 262.0;
/// The grid's top row with the shelf closed; the shelf opens by sliding the grid down.
const GRID_TOP: f32 = 640.0;
const SHELF_SHIFT: f32 = 290.0;
/// Where a focused row below the top settles while scrolling.
const FOCUS_Y: f32 = 330.0;
const SECTION_GAP: f32 = 64.0;
const ROWS_TOP_GAP: f32 = 40.0;
const ROW_TITLE: f32 = 52.0;
const ROW_SPACE: f32 = 80.0;
const RADIUS: f32 = 12.0;
const CARD_RADIUS: f32 = 10.0;
const FOCUS_SCALE: f32 = 1.12;
const CARD_FOCUS_SCALE: f32 = 1.08;
/// The shelf's cards, closed and open.
const CARDS_Y: (f32, f32) = (380.0, 660.0);
const WHITE: Rgba = Rgba::hex(0xFFFFFF);

impl Home {
    // ---- Geometry (shared by navigation, drawing and pointer hits) -------------------

    /// The grid slot in focus: an app, or the Settings tile after the last app.
    pub(super) fn grid_index(&self) -> usize {
        if self.header {
            self.rails[FAVORITES].tiles.len()
        } else {
            self.rails[FAVORITES].focus
        }
    }

    /// Apps plus the Settings tile.
    pub(super) fn grid_total(&self) -> usize {
        self.rails[FAVORITES].tiles.len() + 1
    }

    fn grid_rows(&self) -> usize {
        self.grid_total().div_ceil(COLS)
    }

    /// Top-left of a grid cell before scrolling.
    pub(super) fn grid_xy(col: f32, row: f32) -> (f32, f32) {
        (SAFE_X + col * (TILE.0 + GAP), GRID_TOP + row * ROW_PITCH)
    }

    /// Rails shown as rows under the grid, top to bottom. While TiviOne is missing, its tile
    /// row (the promotion) is one of them; with TiviOne the TV rails live in the shelf.
    pub(super) fn glass_rows(&self) -> impl Iterator<Item = usize> + '_ {
        [TV_RECENT, HOME, INPUTS, HIDDEN].into_iter().filter(move |&r| self.has(r) && (r != TV_RECENT || self.tv.is_none()))
    }

    /// A row's title, before scrolling.
    pub(super) fn glass_row_top(&self, rail: usize) -> f32 {
        let mut y = GRID_TOP + self.grid_rows() as f32 * ROW_PITCH + ROWS_TOP_GAP;
        for r in self.glass_rows() {
            if r == rail {
                return y;
            }
            y += ROW_TITLE + self.rails[r].tile_h + ROW_SPACE;
        }
        y
    }

    /// Bottom of the last row (or of the grid), before scrolling.
    fn glass_content_bottom(&self) -> f32 {
        match self.glass_rows().last() {
            Some(rail) => self.glass_row_top(rail) + ROW_TITLE + self.rails[rail].tile_h,
            None => GRID_TOP + self.grid_rows() as f32 * ROW_PITCH,
        }
    }

    /// What the Top Shelf shows: the content rails of the focused app in the grid's top row.
    pub(super) fn shelf_rails(&self) -> &'static [usize] {
        let favorites = &self.rails[FAVORITES];
        if self.header || favorites.focus >= COLS {
            return &[];
        }
        let Some(app) = favorites.tiles.get(favorites.focus) else { return &[] };
        let rails: &'static [usize] = if (app.id == TIVIONE_APP || app.title == "TiviOne") && self.tv.is_some() {
            &[TV_RECENT, TV_FAVORITES, TV_CATCHUP, TV_COMING]
        } else if app.id == PLEX_APP || app.title == "Plex" {
            &[CONTINUE]
        } else {
            &[]
        };
        if rails.iter().any(|&r| self.has(r)) {
            rails
        } else {
            &[]
        }
    }

    pub(super) fn in_shelf(&self) -> bool {
        !self.header && self.shelf_rails().contains(&self.active)
    }

    /// A card's x in the shelf strip, before shelf scrolling. Sections follow each other.
    pub(super) fn shelf_x(&self, rail: usize, index: usize) -> f32 {
        let mut x = SAFE_X;
        let mut first = true;
        for &r in self.shelf_rails() {
            if !self.has(r) {
                continue;
            }
            if !first {
                x += SECTION_GAP;
            }
            first = false;
            if r == rail {
                return x + index as f32 * (CARD.0 + CARD_GAP);
            }
            x += self.rails[r].tiles.len() as f32 * (CARD.0 + CARD_GAP) - CARD_GAP;
        }
        x
    }

    /// x of a tile's centre in a row under the grid, at its scroll target.
    pub(super) fn row_center(&self, rail: usize, index: usize) -> f32 {
        let r = &self.rails[rail];
        r.slot_x(index as f32) - r.scroll.target + r.tile_w * 0.5
    }

    pub(super) fn glass_menu_anchor(&self) -> (f32, f32) {
        let dy = self.vscroll.value + self.shelf.value * SHELF_SHIFT;
        if self.active == FAVORITES {
            let tile = &self.rails[FAVORITES].tiles.get(self.rails[FAVORITES].focus);
            let (col, row) = tile.map_or((0.0, 0.0), |t| (t.gcol.value, t.grow.value));
            let (x, y) = Self::grid_xy(col, row);
            return (x, y + dy - TILE.1 * (FOCUS_SCALE - 1.0) * 0.5);
        }
        let rail = &self.rails[self.active];
        (rail.slot_x(rail.focus as f32) - rail.scroll.value, self.glass_row_top(self.active) + ROW_TITLE + dy)
    }

    // ---- Focus and animation ---------------------------------------------------------

    pub(super) fn glass_refocus(&mut self, instant: bool) {
        let in_shelf = self.in_shelf();
        self.shelf.target = if in_shelf { 1.0 } else { 0.0 };
        self.settings_focus.target = if self.header { 1.0 } else { 0.0 };
        let in_grid = self.header || self.active == FAVORITES;
        self.vscroll.target = if in_shelf {
            0.0
        } else if in_grid {
            match self.grid_index() / COLS {
                0 => 0.0,
                row => FOCUS_Y - (GRID_TOP + row as f32 * ROW_PITCH),
            }
        } else {
            FOCUS_Y - self.glass_row_top(self.active)
        };
        // Never scroll past the end: the last row stays near the bottom edge.
        let end = (LOGICAL_H - 60.0 - self.glass_content_bottom()).min(0.0);
        self.vscroll.target = self.vscroll.target.max(end);
        if in_shelf {
            // Keep the focused card inside the safe area.
            let x = self.shelf_x(self.active, self.rails[self.active].focus);
            let left = x - self.shelf_scroll.target;
            if left < SAFE_X {
                self.shelf_scroll.target = x - SAFE_X;
            } else if left + CARD.0 > LOGICAL_W - SAFE_X {
                self.shelf_scroll.target = x + CARD.0 - (LOGICAL_W - SAFE_X);
            }
            self.shelf_scroll.target = self.shelf_scroll.target.max(0.0);
        }
        // The backdrop follows the focused app (crossfading); the Settings tile has none.
        let favorites = &self.rails[FAVORITES];
        let app = (!self.header).then(|| favorites.tiles.get(favorites.focus).map(|t| t.id.clone())).flatten();
        if app != self.backdrop {
            self.previous_backdrop = std::mem::replace(&mut self.backdrop, app);
            self.backdrop_t = Spring::new(0.0, 8.0);
            self.backdrop_t.target = 1.0;
            // Another app's shelf starts at its first card.
            self.shelf_scroll = Spring::new(0.0, 16.0);
        }
        if instant {
            for spring in [&mut self.shelf, &mut self.shelf_scroll, &mut self.settings_focus, &mut self.backdrop_t] {
                spring.snap();
            }
        }
    }

    /// Glass-only springs; returns true while any moves.
    pub(super) fn glass_step(&mut self, dt: f32) -> bool {
        let mut moving = false;
        for spring in [&mut self.shelf, &mut self.shelf_scroll, &mut self.settings_focus, &mut self.backdrop_t, &mut self.tilt_x, &mut self.tilt_y] {
            moving |= spring.step(dt);
        }
        if matches!(self.mode, Mode::Move { .. }) {
            self.wiggle += dt;
            moving = true;
        }
        for tile in &mut self.rails[FAVORITES].tiles {
            moving |= tile.gcol.step(dt);
            moving |= tile.grow.step(dt);
        }
        moving
    }

    /// Grid positions follow the list order (after moving an app).
    pub(super) fn glass_reslot(&mut self) {
        let rail = &mut self.rails[FAVORITES];
        rail.reslot();
        for (i, tile) in rail.tiles.iter_mut().enumerate() {
            tile.gcol.target = (i % COLS) as f32;
            tile.grow.target = (i / COLS) as f32;
        }
    }

    // ---- Drawing ---------------------------------------------------------------------

    pub(super) fn draw_glass(&mut self, r: &mut Renderer, text: &mut Text, now: &chrono::DateTime<chrono::Local>) {
        self.hits.borrow_mut().clear();
        let (clock, date) = self.clock_and_date(now);
        let open = self.shelf.value.clamp(0.0, 1.0);
        // The shelf scrolls away with the grid as lower rows take focus.
        let fade = (1.0 + self.vscroll.value / 420.0).clamp(0.0, 1.0);
        let dy = self.vscroll.value + open * SHELF_SHIFT;
        self.glass_backdrop(r, fade, open);
        if fade > 0.01 {
            self.glass_shelf(r, text, fade, open);
        }
        if self.preview.is_some() {
            r.punch(PREVIEW_RECT);
            let pill = Rect::new(PREVIEW_RECT.x + 20.0, PREVIEW_RECT.y + 20.0, 84.0, 36.0);
            r.rect(pill, 8.0, Rgba::hex(0xE5484D));
            text.draw(r, &self.strings.live.to_uppercase(), pill.x + 14.0, pill.y + 26.0, INPUT_SUB, WHITE);
        }
        if open > 0.01 && fade > 0.01 {
            if let Some(hero) = &self.hero {
                let t = self.hero_t.value.clamp(0.0, 1.0);
                draw_hero_block(r, text, hero, open * fade * t, -60.0, WHITE, PREVIEW_RECT.x - SAFE_X - 40.0);
            }
        }
        self.glass_header(r, text, &clock, &date, fade);
        self.glass_grid(r, text, dy);
        self.glass_row_tiles(r, text, dy);
        self.draw_menu(r, text);
        self.draw_setup(r, text);
        self.draw_offer(r, text);
        self.draw_settings(r, text);
        self.draw_store(r, text);
        self.draw_picker(r, text);
        self.draw_digits(r, text);
        self.draw_notice(r, text);
    }

    /// The focused app's artwork across the top, crossfading between apps and fading into the
    /// background; darker while the shelf is open, so its text and video read.
    fn glass_backdrop(&self, r: &mut Renderer, fade: f32, open: f32) {
        if fade <= 0.01 {
            return;
        }
        let area = Rect::new(0.0, 0.0, LOGICAL_W, 780.0);
        let t = self.backdrop_t.value.clamp(0.0, 1.0);
        for (id, weight) in [(&self.previous_backdrop, 1.0 - t), (&self.backdrop, t)] {
            let Some(id) = id else { continue };
            let alpha = weight * fade;
            if alpha <= 0.01 {
                continue;
            }
            match self.arts.get(id) {
                Some((_, [u0, v0, u1, v1])) => {
                    // Full width; the 16:9 artwork is cropped to the band's height.
                    let keep = (16.0 / 9.0) / (area.w / area.h);
                    let (mid, half) = ((v0 + v1) * 0.5, (v1 - v0) * keep * 0.5);
                    r.image(self.art_atlas, area, [*u0, mid - half, *u1, mid + half], 0.0, WHITE.alpha(alpha));
                }
                None => {
                    let color = self.rails[FAVORITES].tiles.iter().find(|t| &t.id == id).map_or(ACCENT_DEFAULT, |t| t.accent);
                    r.radial(1250.0, 260.0, 1100.0, 620.0, color.alpha(0.5 * alpha));
                }
            }
        }
        let bg = GLASS_BG;
        r.vgradient(Rect::new(0.0, 0.0, LOGICAL_W, 220.0), bg.alpha(0.55 * fade), bg.alpha(0.0));
        r.vgradient(Rect::new(0.0, 360.0, LOGICAL_W, 420.0), bg.alpha(0.0), bg);
        if open > 0.01 {
            r.rect(area, 0.0, bg.alpha(0.6 * open * fade));
        }
    }

    /// Clock, date and weather at the top right; they scroll away with the shelf.
    fn glass_header(&self, r: &mut Renderer, text: &mut Text, clock: &str, date: &str, fade: f32) {
        if fade <= 0.01 {
            return;
        }
        let mut x = LOGICAL_W - SAFE_X - text.measure(r, clock, GLASS_CLOCK);
        text.draw(r, clock, x, SAFE_TOP + 36.0, GLASS_CLOCK, WHITE.alpha(0.92 * fade));
        if !date.is_empty() {
            x -= 32.0 + text.measure(r, date, HEADER);
            text.draw(r, date, x, SAFE_TOP + 34.0, HEADER, WHITE.alpha(0.7 * fade));
        }
        if let Some(weather) = self.weather.filter(|_| self.prefs.ui.weather) {
            let temperature = if self.prefs.ui.fahrenheit { weather.temperature * 9.0 / 5.0 + 32.0 } else { weather.temperature };
            let label = format!("{temperature:.0}° · {}", self.strings.weather(weather.code));
            x -= 32.0 + text.measure(r, &label, HEADER);
            text.draw(r, &label, x, SAFE_TOP + 34.0, HEADER, WHITE.alpha(0.7 * fade));
        }
    }

    /// The focused app's Top Shelf content: section titles over cards in one strip.
    fn glass_shelf(&self, r: &mut Renderer, text: &mut Text, fade: f32, open: f32) {
        let rails = self.shelf_rails();
        if rails.is_empty() {
            return;
        }
        let cards_y = CARDS_Y.0 + (CARDS_Y.1 - CARDS_Y.0) * open + self.vscroll.value;
        let browse = matches!(self.mode, Mode::Browse);
        for pass in 0..4 {
            for &rail in rails {
                if !self.has(rail) {
                    continue;
                }
                let tiles = &self.rails[rail].tiles;
                if pass == 3 {
                    let x = self.shelf_x(rail, 0) - self.shelf_scroll.value;
                    text.draw_top(r, self.rail_title(rail), x, cards_y - 46.0, GLASS_LABEL, WHITE.alpha(0.8 * fade));
                }
                for (i, tile) in tiles.iter().enumerate() {
                    let x = self.shelf_x(rail, i) - self.shelf_scroll.value;
                    if x > LOGICAL_W || x + CARD.0 < -40.0 {
                        continue;
                    }
                    let f = tile.focus.value.clamp(0.0, 1.0);
                    let rect = Rect::new(x, cards_y, CARD.0, CARD.1).scale_about(1.0 + (CARD_FOCUS_SCALE - 1.0) * f, 0.5, 0.5);
                    if pass == 0 && browse {
                        self.hits.borrow_mut().push((rect, Target::Tile { rail, index: i }));
                    }
                    self.glass_tile(r, text, pass, tile, rect, f, fade, CARD_RADIUS);
                }
            }
        }
    }

    fn glass_grid(&self, r: &mut Renderer, text: &mut Text, dy: f32) {
        let favorites = &self.rails[FAVORITES];
        let browse = matches!(self.mode, Mode::Browse);
        let lift = self.lift.value.clamp(0.0, 1.0);
        let n = favorites.tiles.len();
        for pass in 0..4 {
            for (i, tile) in favorites.tiles.iter().enumerate() {
                let (x, y) = Self::grid_xy(tile.gcol.value, tile.grow.value);
                let base = Rect::new(x, y + dy, TILE.0, TILE.1);
                if base.y > LOGICAL_H || base.y + base.h < -80.0 {
                    continue;
                }
                let f = tile.focus.value.clamp(0.0, 1.0);
                let lifted = if i == favorites.focus { lift } else { 0.0 };
                let rect = base.scale_about(1.0 + (FOCUS_SCALE - 1.0) * f.max(lifted), 0.5, 0.5);
                if pass == 0 && browse {
                    self.hits.borrow_mut().push((rect, Target::Tile { rail: FAVORITES, index: i }));
                }
                self.wiggle_tile(r, rect, i);
                self.glass_tile(r, text, pass, tile, rect, f.max(lifted), 1.0, RADIUS);
                r.set_rotation(None);
            }
            // The Settings tile, in the slot after the last app.
            let (x, y) = Self::grid_xy((n % COLS) as f32, (n / COLS) as f32);
            let f = self.settings_focus.value.clamp(0.0, 1.0);
            let rect = Rect::new(x, y + dy, TILE.0, TILE.1).scale_about(1.0 + (FOCUS_SCALE - 1.0) * f, 0.5, 0.5);
            if rect.y > LOGICAL_H || rect.y + rect.h < -80.0 {
                continue;
            }
            self.wiggle_tile(r, rect, n);
            match pass {
                0 => {
                    if browse {
                        self.hits.borrow_mut().push((rect, Target::Gear));
                    }
                    if f > 0.01 {
                        r.shadow(rect.offset(0.0, 18.0 * f), RADIUS, 44.0, Rgba::hex(0).alpha(0.6 * f));
                    }
                    r.rect(rect, RADIUS, Rgba::hex(0x3C3C43));
                    draw_gear_at(r, rect.x + rect.w * 0.5, rect.y + rect.h * 0.5, 2.6 * rect.h / TILE.1, Rgba::hex(0xE6E6EA));
                }
                // A soft light from above, brighter (and following the tilt) when focused.
                1 => {
                    let (tx, ty) = (self.tilt_x.value * f, self.tilt_y.value * f);
                    let uv = sheen_uv(rect, 0.5 + 0.3 * tx, 0.1 + 0.3 * ty);
                    r.image(self.art_atlas, rect, uv, RADIUS, WHITE.alpha(0.14 + 0.12 * f));
                }
                3 if f > 0.01 => {
                    let label = self.strings.settings;
                    let w = text.measure(r, label, GLASS_LABEL);
                    text.draw_top(r, label, rect.x + (rect.w - w) * 0.5, rect.y + rect.h + 16.0, GLASS_LABEL, WHITE.alpha(f));
                }
                _ => {}
            }
            r.set_rotation(None);
        }
    }

    /// While apps are being moved, every grid tile rocks a little, each out of step.
    fn wiggle_tile(&self, r: &mut Renderer, rect: Rect, index: usize) {
        if matches!(self.mode, Mode::Move { .. }) {
            let angle = 0.024 * (self.wiggle * 15.0 + index as f32 * 1.7).sin();
            r.set_rotation(Some((rect.x + rect.w * 0.5, rect.y + rect.h * 0.5, angle)));
        }
    }

    /// Home Assistant, inputs, hidden apps (and the TiviOne promotion) under the grid.
    fn glass_row_tiles(&self, r: &mut Renderer, text: &mut Text, dy: f32) {
        let browse = matches!(self.mode, Mode::Browse);
        for pass in 0..4 {
            for rail in self.glass_rows() {
                let top = self.glass_row_top(rail) + dy;
                if top > LOGICAL_H || top + 400.0 < 0.0 {
                    continue;
                }
                if pass == 3 {
                    let dim = if rail == self.active { 0.9 } else { 0.6 };
                    text.draw_top(r, self.rail_title(rail), SAFE_X, top, GLASS_SECTION, WHITE.alpha(dim));
                }
                let row = &self.rails[rail];
                for (i, tile) in row.tiles.iter().enumerate() {
                    let base = Rect::new(row.slot_x(tile.slot.value) - row.scroll.value, top + ROW_TITLE, row.tile_w, row.tile_h);
                    if base.x > LOGICAL_W || base.x + base.w < -40.0 {
                        continue;
                    }
                    let f = tile.focus.value.clamp(0.0, 1.0);
                    let rect = base.scale_about(1.0 + (CARD_FOCUS_SCALE - 1.0) * f, 0.5, 0.5);
                    if pass == 0 && browse {
                        self.hits.borrow_mut().push((rect, Target::Tile { rail, index: i }));
                    }
                    self.glass_tile(r, text, pass, tile, rect, f, 1.0, RADIUS);
                }
            }
        }
    }

    /// One tile in one pass (shapes → artwork → icons → text keeps texture switches low).
    #[allow(clippy::too_many_arguments)]
    fn glass_tile(&self, r: &mut Renderer, text: &mut Text, pass: usize, tile: &Tile, rect: Rect, f: f32, alpha: f32, radius: f32) {
        let view = TileView {
            now: if pass == 3 { self.channel_now(tile) } else { None },

            rect,
            f,
            alpha,
            lifted: 0.0,
            icon: self.icons.get(&tile.id).copied(),
            art: self.arts.get(&tile.id).map(|(_, uv)| *uv),
            fill: self.blended.get(&tile.id).copied(),
            radius,
            ring: false,
        };
        // Parallax on the focused tile: the shadow falls away from the tilt, the artwork shifts
        // inside its frame and a highlight sweeps across.
        let (tx, ty) = (self.tilt_x.value * f, self.tilt_y.value * f);
        match pass {
            0 => {
                if f > 0.01 {
                    r.shadow(rect.offset(-10.0 * tx, 18.0 * f - 6.0 * ty), radius, 44.0, Rgba::hex(0).alpha(0.6 * f * alpha));
                }
                draw_tile_shapes(r, tile, &view);
            }
            1 => {
                match view.art {
                    Some([u0, v0, u1, v1]) if f > 0.01 => {
                        let zoom = 1.0 + 0.06 * f;
                        let (hw, hh) = ((u1 - u0) * 0.5 / zoom, (v1 - v0) * 0.5 / zoom);
                        let (cu, cv) = ((u0 + u1) * 0.5 + tx * ((u1 - u0) * 0.5 - hw), (v0 + v1) * 0.5 + ty * ((v1 - v0) * 0.5 - hh));
                        draw_tile_art(r, self.art_atlas, &TileView { art: Some([cu - hw, cv - hh, cu + hw, cv + hh]), ..view });
                    }
                    _ => draw_tile_art(r, self.art_atlas, &view),
                }
                if f > 0.01 {
                    r.image(self.art_atlas, rect, sheen_uv(rect, 0.5 + 0.3 * tx, 0.3 + 0.3 * ty), radius, WHITE.alpha(0.2 * f * alpha));
                }
            }
            2 => draw_tile_icon(r, self.icon_atlas, &view),
            _ => {
                draw_tile_text(r, text, tile, &view, self.strings);
                // Only the focused tile shows its title, under it.
                if f > 0.01 && matches!(tile.kind, Kind::App | Kind::Media { .. }) {
                    let title = clip(text, r, &tile.title, GLASS_LABEL, rect.w + 60.0);
                    let w = text.measure(r, &title, GLASS_LABEL);
                    text.draw_top(r, &title, rect.x + (rect.w - w) * 0.5, rect.y + rect.h + 16.0, GLASS_LABEL, WHITE.alpha(f * alpha));
                }
            }
        }
    }
}

/// The sheen texture: a soft white disc in the art atlas's unused strip (x 1920–2048).
const SHEEN: (i32, i32, i32) = (1920, 0, 128);

pub(super) fn upload_sheen(renderer: &Renderer, atlas: Texture) {
    let (_, _, size) = SHEEN;
    let half = size as f32 * 0.5;
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let d = ((x as f32 + 0.5 - half).hypot(y as f32 + 0.5 - half) / half).min(1.0);
            pixels.extend_from_slice(&[255, 255, 255, ((1.0 - d) * (1.0 - d) * 255.0) as u8]);
        }
    }
    renderer.upload(atlas, SHEEN.0, SHEEN.1, size, size, gl::RGBA, &pixels);
}

/// UVs that put the sheen's centre at (fx, fy) of `rect` (fractions), with a radius of about
/// 0.8 of its width; the window stays inside the strip for fx in 0.2..0.8.
fn sheen_uv(rect: Rect, fx: f32, fy: f32) -> [f32; 4] {
    let (x0, y0, size) = (SHEEN.0 as f32, SHEEN.1 as f32, SHEEN.2 as f32);
    let scale = size * 0.5 / (rect.w * 0.8);
    let (w, h) = (rect.w * scale, rect.h * scale);
    let u0 = (x0 + size * 0.5 - fx.clamp(0.2, 0.8) * w).clamp(x0, x0 + size - w);
    let v0 = (y0 + size * 0.5 - fy.clamp(0.0, 1.0) * h).clamp(y0, y0 + size - h);
    let atlas = ART_ATLAS as f32;
    [u0 / atlas, v0 / atlas, (u0 + w) / atlas, (v0 + h) / atlas]
}
