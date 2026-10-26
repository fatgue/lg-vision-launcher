//! The Glass look: a photo wallpaper, the first row of apps in a frosted dock, more rows
//! below it, Settings behind the gear at the top left and a large clock at the top right.
//! When TiviOne or Plex has focus in the dock, its content appears above the dock (the Top
//! Shelf); Up opens it with programme details and the live preview.
use super::cinematic::*;
use super::*;

pub(super) const COLS: usize = 6;
pub(super) const TILE: (f32, f32) = (255.0, 143.0);
pub(super) const GAP: f32 = 48.0;
pub(super) const CARD: (f32, f32) = (285.0, 160.0);
pub(super) const CARD_GAP: f32 = 28.0;
pub(super) const ROW_GAP: f32 = 40.0;
/// Left edge of the first column; six tiles and five gaps end at 1845.
pub(super) const X0: f32 = 75.0;
/// The dock holding the first row, and that row's top.
const DOCK: Rect = Rect::new(25.0, 782.0, 1870.0, 218.0);
const DOCK_RADIUS: f32 = 44.0;
const ROW0_Y: f32 = 819.0;
/// Rows after the first, below the dock.
const LOWER_TOP: f32 = 1036.0;
const ROW_PITCH: f32 = 200.0;
/// Where a focused lower row settles while scrolling.
const FOCUS_Y: f32 = 560.0;
const ROWS_TOP_GAP: f32 = 30.0;
const ROW_TITLE: f32 = 52.0;
const ROW_SPACE: f32 = 70.0;
const RADIUS: f32 = 14.0;
const CARD_RADIUS: f32 = 12.0;
const FOCUS_SCALE: f32 = 1.18;
const CARD_FOCUS_SCALE: f32 = 1.1;
/// The shelf's cards sit just above the dock.
const CARDS_Y: f32 = 600.0;
const SECTION_GAP: f32 = 48.0;
/// The gear (Settings) at the top left.
const GEAR: (f32, f32) = (66.0, 60.0);
const WHITE: Rgba = Rgba::hex(0xFFFFFF);
const BLACK: Rgba = Rgba::hex(0x000000);

impl Home {
    // ---- Geometry (shared by navigation, drawing and pointer hits) -------------------

    pub(super) fn grid_index(&self) -> usize {
        self.rails[FAVORITES].focus
    }

    pub(super) fn grid_total(&self) -> usize {
        self.rails[FAVORITES].tiles.len()
    }

    fn grid_rows(&self) -> usize {
        self.grid_total().max(1).div_ceil(COLS)
    }

    /// Top-left of a grid cell before scrolling: row 0 sits in the dock, later rows below it.
    pub(super) fn grid_xy(col: f32, row: f32) -> (f32, f32) {
        let y = if row <= 1.0 { ROW0_Y + row * (LOWER_TOP - ROW0_Y) } else { LOWER_TOP + (row - 1.0) * ROW_PITCH };
        (X0 + col * (TILE.0 + GAP), y)
    }

    /// Rails shown as rows under the grid, top to bottom. While TiviOne is missing, its tile
    /// row (the promotion) is one of them; with TiviOne the TV rails live in the shelf.
    pub(super) fn glass_rows(&self) -> impl Iterator<Item = usize> + '_ {
        [TV_RECENT, HOME, INPUTS, HIDDEN].into_iter().filter(move |&r| self.has(r) && (r != TV_RECENT || self.tv.is_none()))
    }

    /// A row's title, before scrolling.
    pub(super) fn glass_row_top(&self, rail: usize) -> f32 {
        let last = Self::grid_xy(0.0, (self.grid_rows() - 1) as f32).1;
        // Under the dock when the grid is a single row, else under the last row.
        let mut y = if self.grid_rows() == 1 { DOCK.y + DOCK.h } else { last + TILE.1 } + ROWS_TOP_GAP;
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
            None => Self::grid_xy(0.0, (self.grid_rows() - 1) as f32).1 + TILE.1,
        }
    }

    /// What the Top Shelf shows: the content rails of the focused app in the dock.
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
        let mut x = X0;
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
        let dy = self.vscroll.value;
        if self.active == FAVORITES {
            let tile = self.rails[FAVORITES].tiles.get(self.rails[FAVORITES].focus);
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
        let in_grid = self.header || self.active == FAVORITES || in_shelf;
        self.vscroll.target = if in_grid {
            match self.grid_index() / COLS {
                row if row == 0 || self.header || in_shelf => 0.0,
                row => FOCUS_Y - Self::grid_xy(0.0, row as f32).1,
            }
        } else {
            FOCUS_Y - self.glass_row_top(self.active)
        };
        // Never scroll past the end: the last row stays near the bottom edge.
        let end = (LOGICAL_H - 60.0 - self.glass_content_bottom()).min(0.0);
        self.vscroll.target = self.vscroll.target.max(end);
        if in_shelf {
            // Keep the focused card inside the screen.
            let x = self.shelf_x(self.active, self.rails[self.active].focus);
            let left = x - self.shelf_scroll.target;
            if left < X0 {
                self.shelf_scroll.target = x - X0;
            } else if left + CARD.0 > LOGICAL_W - X0 {
                self.shelf_scroll.target = x + CARD.0 - (LOGICAL_W - X0);
            }
            self.shelf_scroll.target = self.shelf_scroll.target.max(0.0);
        }
        // Another app's shelf starts at its first card.
        let favorites = &self.rails[FAVORITES];
        let app = favorites.tiles.get(favorites.focus).map(|t| t.id.clone());
        if app != self.backdrop {
            self.backdrop = app;
            self.shelf_scroll = Spring::new(0.0, 16.0);
        }
        if instant {
            for spring in [&mut self.shelf, &mut self.shelf_scroll, &mut self.settings_focus] {
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

    /// Takes a decoded wallpaper: two textures, created on first use and refilled after.
    pub fn set_wallpaper(&mut self, wallpaper: Option<crate::model::Wallpaper>, renderer: &Renderer) {
        let Some(wallpaper) = wallpaper else { return };
        let (w, h) = crate::model::WALLPAPER;
        let (bw, bh) = crate::model::WALLPAPER_BLUR;
        let (full, blur) = *self
            .wallpaper
            .get_or_insert_with(|| (renderer.create_texture(w as i32, h as i32, gl::RGB), renderer.create_texture(bw as i32, bh as i32, gl::RGB)));
        renderer.upload(full, 0, 0, w as i32, h as i32, gl::RGB, &wallpaper.rgb);
        renderer.upload(blur, 0, 0, bw as i32, bh as i32, gl::RGB, &wallpaper.blur);
        self.backdrop_t = Spring::new(0.0, 6.0);
        self.backdrop_t.target = 1.0;
    }

    // ---- Drawing ---------------------------------------------------------------------

    pub(super) fn draw_glass(&mut self, r: &mut Renderer, text: &mut Text, now: &chrono::DateTime<chrono::Local>) {
        self.hits.borrow_mut().clear();
        let (clock, date) = self.clock_and_date(now);
        let v = self.vscroll.value;
        let open = self.shelf.value.clamp(0.0, 1.0);
        // The header and the shelf scroll away as lower rows take focus.
        let fade = (1.0 + v / 360.0).clamp(0.0, 1.0);
        self.glass_wallpaper(r, fade, open);
        if fade > 0.01 {
            self.glass_shelf(r, text, fade);
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
                draw_hero_block(r, text, hero, open * fade * t, -60.0, WHITE, PREVIEW_RECT.x - X0 - 40.0);
            }
        }
        self.glass_header(r, text, &clock, &date, fade);
        self.glass_dock(r, v);
        self.glass_grid(r, text, v);
        self.glass_row_tiles(r, text, v);
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

    /// The photo, darkened as lower rows scroll up and behind the shelf's content.
    fn glass_wallpaper(&self, r: &mut Renderer, fade: f32, open: f32) {
        let screen = Rect::new(0.0, 0.0, LOGICAL_W, LOGICAL_H);
        match self.wallpaper {
            Some((full, _)) => r.image(full, screen, [0.0, 0.0, 1.0, 1.0], 0.0, WHITE.alpha(self.backdrop_t.value.clamp(0.0, 1.0))),
            None => r.vgradient(screen, Rgba::hex(0x1B2330), GLASS_BG),
        }
        // Legibility: the clock at the top, the shelf above the dock, the rows when scrolled.
        r.vgradient(Rect::new(0.0, 0.0, LOGICAL_W, 200.0), BLACK.alpha(0.35), BLACK.alpha(0.0));
        if !self.shelf_rails().is_empty() {
            r.vgradient(Rect::new(0.0, 360.0, LOGICAL_W, 420.0), BLACK.alpha(0.0), BLACK.alpha(0.55 * fade));
        }
        let dim = 0.5 * open * fade + (1.0 - fade) * 0.6;
        if dim > 0.0 {
            r.rect(screen, 0.0, BLACK.alpha(dim));
        }
    }

    /// The gear (Settings) at the top left; the clock, date and weather at the top right.
    fn glass_header(&self, r: &mut Renderer, text: &mut Text, clock: &str, date: &str, fade: f32) {
        if fade <= 0.01 {
            return;
        }
        let f = self.settings_focus.value.clamp(0.0, 1.0);
        let (gx, gy) = GEAR;
        if matches!(self.mode, Mode::Browse) {
            self.hits.borrow_mut().push((Rect::new(gx - 36.0, gy - 36.0, 72.0, 72.0), Target::Gear));
        }
        if f > 0.01 {
            r.rect(Rect::new(gx - 34.0, gy - 34.0, 68.0, 68.0), 34.0, WHITE.alpha(f * fade));
        }
        draw_gear_at(r, gx, gy, 1.3, WHITE.lerp(Rgba::hex(0x111114), f).alpha(fade));

        let shadowed = |text: &mut Text, r: &mut Renderer, s: &str, x: f32, y: f32, style: Style, alpha: f32| {
            text.draw(r, s, x, y + 2.0, style, BLACK.alpha(0.35 * alpha));
            text.draw(r, s, x, y, style, WHITE.alpha(alpha));
        };
        let mut x = LOGICAL_W - 64.0 - text.measure(r, clock, GLASS_CLOCK);
        shadowed(text, r, clock, x, 76.0, GLASS_CLOCK, fade);
        if !date.is_empty() {
            x -= 30.0 + text.measure(r, date, HEADER);
            shadowed(text, r, date, x, 72.0, HEADER, 0.85 * fade);
        }
        if let Some(weather) = self.weather.filter(|_| self.prefs.ui.weather) {
            let temperature = if self.prefs.ui.fahrenheit { weather.temperature * 9.0 / 5.0 + 32.0 } else { weather.temperature };
            let label = format!("{temperature:.0}° · {}", self.strings.weather(weather.code));
            x -= 30.0 + text.measure(r, &label, HEADER);
            shadowed(text, r, &label, x, 72.0, HEADER, 0.85 * fade);
        }
    }

    /// Frosted glass: the blurred copy of the wallpaper behind the dock, lightly tinted.
    fn glass_dock(&self, r: &mut Renderer, dy: f32) {
        let dock = DOCK.offset(0.0, dy);
        if dock.y > LOGICAL_H || dock.y + dock.h < 0.0 {
            return;
        }
        r.shadow(dock.offset(0.0, 12.0), DOCK_RADIUS, 40.0, BLACK.alpha(0.25));
        if let Some((_, blur)) = self.wallpaper {
            let uv = [dock.x / LOGICAL_W, dock.y / LOGICAL_H, (dock.x + dock.w) / LOGICAL_W, (dock.y + dock.h) / LOGICAL_H];
            r.image(blur, dock, uv, DOCK_RADIUS, WHITE.alpha(self.backdrop_t.value.clamp(0.0, 1.0)));
        }
        r.rect(dock, DOCK_RADIUS, WHITE.alpha(0.14));
        r.stroke(dock.inflate(-0.75), DOCK_RADIUS, 1.5, WHITE.alpha(0.3));
    }

    /// The focused app's shelf content: section titles over cards, just above the dock.
    fn glass_shelf(&self, r: &mut Renderer, text: &mut Text, fade: f32) {
        let rails = self.shelf_rails();
        if rails.is_empty() {
            return;
        }
        let cards_y = CARDS_Y + self.vscroll.value;
        let browse = matches!(self.mode, Mode::Browse);
        for pass in 0..4 {
            for &rail in rails {
                if !self.has(rail) {
                    continue;
                }
                if pass == 3 {
                    let x = self.shelf_x(rail, 0) - self.shelf_scroll.value;
                    text.draw_top(r, self.rail_title(rail), x, cards_y - 42.0, GLASS_LABEL, WHITE.alpha(0.85 * fade));
                }
                for (i, tile) in self.rails[rail].tiles.iter().enumerate() {
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
        for pass in 0..4 {
            for (i, tile) in favorites.tiles.iter().enumerate() {
                let (x, y) = Self::grid_xy(tile.gcol.value, tile.grow.value);
                let base = Rect::new(x, y + dy, TILE.0, TILE.1);
                if base.y > LOGICAL_H || base.y + base.h < -80.0 {
                    continue;
                }
                let f = tile.focus.value.clamp(0.0, 1.0).max(if i == favorites.focus { lift } else { 0.0 });
                let rect = base.scale_about(1.0 + (FOCUS_SCALE - 1.0) * f, 0.5, 0.5);
                if pass == 0 && browse {
                    self.hits.borrow_mut().push((rect, Target::Tile { rail: FAVORITES, index: i }));
                }
                self.wiggle_tile(r, rect, i);
                self.glass_tile(r, text, pass, tile, rect, f, 1.0, RADIUS);
                r.set_rotation(None);
            }
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
                    let dim = if rail == self.active { 0.95 } else { 0.7 };
                    text.draw_top(r, self.rail_title(rail), X0, top + 2.0, GLASS_SECTION, BLACK.alpha(0.35 * dim));
                    text.draw_top(r, self.rail_title(rail), X0, top, GLASS_SECTION, WHITE.alpha(dim));
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
    /// Focus: larger, a white frame and a soft shadow, the artwork shifting with the tilt and
    /// a highlight sweeping across.
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
        let (tx, ty) = (self.tilt_x.value * f, self.tilt_y.value * f);
        match pass {
            0 => {
                if f > 0.01 {
                    r.shadow(rect.offset(-10.0 * tx, 16.0 * f - 6.0 * ty), radius, 40.0, BLACK.alpha(0.55 * f * alpha));
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
                    r.image(self.art_atlas, rect, sheen_uv(rect, 0.5 + 0.3 * tx, 0.3 + 0.3 * ty), radius, WHITE.alpha(0.18 * f * alpha));
                    r.stroke(rect.inflate(1.5), radius + 1.5, 3.0, WHITE.alpha(f * alpha));
                }
            }
            2 => draw_tile_icon(r, self.icon_atlas, &view),
            _ => draw_tile_text(r, text, tile, &view, self.strings),
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
