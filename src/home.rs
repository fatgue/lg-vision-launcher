//! Home screen state, input and drawing (direction A, "Cinematic Ambient").
use crate::anim::Spring;
use crate::color::Rgba;
use crate::gl;
use crate::model::{App, Icon, Input, Notice};
use crate::render::{Rect, Renderer, Texture, LOGICAL_H, LOGICAL_W};
use crate::storage::Prefs;
use crate::strings::{self, Strings};
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

const FAVORITES: usize = 0;
const INPUTS: usize = 1;
const HIDDEN: usize = 2;

pub enum Action {
    Launch { app_id: String, params: Option<Value> },
    SavePrefs(Prefs),
    Exit,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Ok,
    /// OK held past the long-press threshold (detected by the event loop).
    LongOk,
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
    focus: Spring,
    /// Slot position in tile units; animates when tiles are moved or hidden.
    slot: Spring,
}

impl Tile {
    fn app(app: App, index: usize) -> Self {
        let mut tile = Self {
            id: app.id.clone(),
            title: app.title,
            launch_id: app.id,
            params: app.params,
            kind: Kind::App,
            bg: TILE_NEUTRAL,
            accent: ACCENT_DEFAULT,
            focus: Spring::new(0.0, 18.0),
            slot: Spring::new(index as f32, 16.0),
        };
        if let Some(color) = app.bg_color.as_deref().and_then(Rgba::parse) {
            tile.tint(color);
        }
        tile
    }

    fn tint(&mut self, color: Rgba) {
        self.bg = BG.lerp(color, 0.2);
        self.accent = color.luminous(0.92);
    }
}

struct Rail {
    tiles: Vec<Tile>,
    focus: usize,
    scroll: Spring,
    tile_w: f32,
    tile_h: f32,
}

impl Rail {
    fn new(tile_w: f32, tile_h: f32) -> Self {
        Self { tiles: Vec::new(), focus: 0, scroll: Spring::new(0.0, 16.0), tile_w, tile_h }
    }

    fn slot_x(&self, slot: f32) -> f32 {
        SAFE_X + slot * (self.tile_w + TILE_GAP)
    }

    /// Minimal scroll that keeps the focused tile inside the safe area.
    fn follow_focus(&mut self) {
        let x = self.slot_x(self.focus as f32);
        let left = x - self.scroll.target;
        if left < SAFE_X {
            self.scroll.target = x - SAFE_X;
        } else if left + self.tile_w > LOGICAL_W - SAFE_X {
            self.scroll.target = x + self.tile_w - (LOGICAL_W - SAFE_X);
        }
        self.scroll.target = self.scroll.target.max(0.0);
    }

    fn reslot(&mut self) {
        for (i, tile) in self.tiles.iter_mut().enumerate() {
            tile.slot.target = i as f32;
        }
    }

    fn ids(&self) -> Vec<String> {
        self.tiles.iter().map(|t| t.id.clone()).collect()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum MenuItem {
    Move,
    Hide,
    Show,
    Cancel,
}

struct Menu {
    items: Vec<MenuItem>,
    focus: usize,
}

enum Mode {
    Browse,
    Menu(Menu),
    Move { original: Vec<String> },
}

struct Hero {
    eyebrow: String,
    title: String,
    sub: String,
    hint: Option<&'static str>,
}

pub struct Home {
    strings: &'static Strings,
    rails: [Rail; 3],
    active: usize,
    mode: Mode,
    prefs: Prefs,
    apps: Vec<App>,
    vscroll: Spring,
    overlay: Spring,
    lift: Spring,
    ambient: Rgba,
    hero: Option<Hero>,
    previous_hero: Option<Hero>,
    hero_t: Spring,
    icons: HashMap<String, [f32; 5]>,
    colors: HashMap<String, Rgba>,
    icon_atlas: Texture,
    icon_next: i32,
    notice: Option<(String, Instant)>,
    pub loaded: bool,
}

impl Home {
    pub fn new(renderer: &Renderer, prefs: Prefs) -> Self {
        Self {
            strings: strings::current(),
            rails: [Rail::new(300.0, 170.0), Rail::new(300.0, 120.0), Rail::new(300.0, 170.0)],
            active: FAVORITES,
            mode: Mode::Browse,
            prefs,
            apps: Vec::new(),
            vscroll: Spring::new(0.0, 14.0),
            overlay: Spring::new(0.0, 22.0),
            lift: Spring::new(0.0, 18.0),
            ambient: ACCENT_DEFAULT,
            hero: None,
            previous_hero: None,
            hero_t: Spring::new(1.0, 14.0),
            icons: HashMap::new(),
            colors: HashMap::new(),
            icon_atlas: renderer.create_texture(ICON_ATLAS, ICON_ATLAS, gl::RGBA),
            icon_next: 0,
            notice: None,
            loaded: false,
        }
    }

    fn rail_title(&self, rail: usize) -> &'static str {
        match rail {
            FAVORITES => self.strings.favorites,
            INPUTS => self.strings.inputs,
            _ => self.strings.hidden,
        }
    }

    pub fn set_apps(&mut self, apps: Vec<App>) {
        self.apps = apps;
        self.rebuild_app_rails();
    }

    /// Splits the LG launch points into favourites (user order, then LG order) and hidden apps.
    fn rebuild_app_rails(&mut self) {
        let focused_id = self.focused().map(|t| t.id.clone());
        let by_id: HashMap<&str, &App> = self.apps.iter().map(|a| (a.id.as_str(), a)).collect();
        let hidden: Vec<&App> = self.prefs.hidden.iter().filter_map(|id| by_id.get(id.as_str()).copied()).collect();
        let is_hidden = |id: &str| self.prefs.hidden.iter().any(|h| h == id);
        let mut favorites: Vec<&App> = self
            .prefs
            .order
            .iter()
            .filter_map(|id| by_id.get(id.as_str()).copied())
            .filter(|app| !is_hidden(&app.id))
            .collect();
        for app in &self.apps {
            if !is_hidden(&app.id) && !favorites.iter().any(|f| f.id == app.id) {
                favorites.push(app);
            }
        }
        let build = |list: Vec<&App>, colors: &HashMap<String, Rgba>| -> Vec<Tile> {
            list.into_iter()
                .enumerate()
                .map(|(i, app)| {
                    let mut tile = Tile::app(app.clone(), i);
                    if let Some(color) = colors.get(&app.id) {
                        tile.tint(*color);
                    }
                    tile
                })
                .collect()
        };
        let favorites = build(favorites, &self.colors);
        let hidden = build(hidden, &self.colors);
        self.rails[FAVORITES].tiles = favorites;
        self.rails[HIDDEN].tiles = hidden;
        self.mode = Mode::Browse;
        self.restore_focus(focused_id);
        self.loaded = true;
        self.refocus(true);
    }

    fn restore_focus(&mut self, id: Option<String>) {
        for rail in [FAVORITES, HIDDEN] {
            let r = &mut self.rails[rail];
            r.focus = r.focus.min(r.tiles.len().saturating_sub(1));
            if let Some(index) = id.as_ref().and_then(|id| r.tiles.iter().position(|t| &t.id == id)) {
                r.focus = index;
                self.active = rail;
            }
        }
        if self.rails[self.active].tiles.is_empty() {
            self.active = FAVORITES;
        }
    }

    pub fn set_inputs(&mut self, inputs: Vec<Input>) {
        self.rails[INPUTS].tiles = inputs
            .into_iter()
            .enumerate()
            .map(|(i, input)| Tile {
                id: input.id,
                title: input.label,
                launch_id: input.app_id,
                params: None,
                kind: Kind::Input { connected: input.connected },
                bg: Rgba::hex(0x14161C),
                accent: ACCENT_INPUT,
                focus: Spring::new(0.0, 18.0),
                slot: Spring::new(i as f32, 16.0),
            })
            .collect();
        let rail = &mut self.rails[INPUTS];
        rail.focus = rail.focus.min(rail.tiles.len().saturating_sub(1));
        self.refocus(true);
    }

    /// Packs an icon into the shared atlas once and tints every tile of that app.
    pub fn add_icon(&mut self, icon: Icon, renderer: &Renderer) {
        if let Some(color) = icon.color {
            let color = Rgba::from_u8(color);
            self.colors.insert(icon.app_id.clone(), color);
            for rail in [FAVORITES, HIDDEN] {
                for tile in self.rails[rail].tiles.iter_mut().filter(|t| t.id == icon.app_id) {
                    tile.tint(color);
                }
            }
        }
        if self.icons.contains_key(&icon.app_id) {
            return;
        }
        let per_row = ICON_ATLAS / ICON_CELL;
        if self.icon_next >= per_row * per_row {
            return;
        }
        let (cx, cy) = ((self.icon_next % per_row) * ICON_CELL, (self.icon_next / per_row) * ICON_CELL);
        self.icon_next += 1;
        renderer.upload(self.icon_atlas, cx, cy, icon.w as i32, icon.h as i32, gl::RGBA, &icon.pixels);
        let a = ICON_ATLAS as f32;
        self.icons.insert(
            icon.app_id,
            [cx as f32 / a, cy as f32 / a, (cx + icon.w as i32) as f32 / a, (cy + icon.h as i32) as f32 / a, icon.w as f32 / icon.h as f32],
        );
    }

    pub fn notice(&mut self, notice: Notice) {
        let s = self.strings;
        let text = match notice {
            Notice::ServiceUnavailable => s.notice_service.to_string(),
            Notice::AppListUnavailable => s.notice_app_list.to_string(),
            Notice::LaunchFailed => s.notice_launch_failed.to_string(),
            Notice::PreviewLaunch(app) => format!("{} {app}", s.notice_preview_launch),
        };
        self.notice = Some((text, Instant::now()));
    }

    fn has(&self, rail: usize) -> bool {
        self.rails.get(rail).is_some_and(|r| !r.tiles.is_empty())
    }

    /// Vertical slot of a rail among the non-empty ones.
    fn rail_slot(&self, rail: usize) -> usize {
        (0..rail).filter(|&r| self.has(r)).count()
    }

    pub fn key(&mut self, key: Key, home_mode: bool) -> Option<Action> {
        let action = match &self.mode {
            Mode::Menu(_) => self.menu_key(key),
            Mode::Move { .. } => self.move_key(key),
            Mode::Browse => self.browse_key(key, home_mode),
        };
        self.refocus(false);
        action
    }

    fn browse_key(&mut self, key: Key, home_mode: bool) -> Option<Action> {
        let active = self.active;
        let (focus, count) = (self.rails[active].focus, self.rails[active].tiles.len());
        match key {
            Key::Left if focus > 0 => self.rails[active].focus -= 1,
            Key::Right if focus + 1 < count => self.rails[active].focus += 1,
            Key::Up => {
                if let Some(rail) = (0..active).rev().find(|&r| self.has(r)) {
                    self.active = rail;
                }
            }
            Key::Down => {
                if let Some(rail) = (active + 1..self.rails.len()).find(|&r| self.has(r)) {
                    self.active = rail;
                }
            }
            Key::Ok => {
                let tile = self.focused()?;
                return Some(Action::Launch { app_id: tile.launch_id.clone(), params: tile.params.clone() });
            }
            Key::LongOk if active == FAVORITES && count > 0 => {
                self.mode = Mode::Menu(Menu { items: vec![MenuItem::Move, MenuItem::Hide, MenuItem::Cancel], focus: 0 });
            }
            Key::LongOk if active == HIDDEN && count > 0 => {
                self.mode = Mode::Menu(Menu { items: vec![MenuItem::Show, MenuItem::Cancel], focus: 0 });
            }
            Key::Back if focus > 0 => self.rails[active].focus = 0,
            Key::Back if active != FAVORITES => self.active = FAVORITES,
            Key::Back if !home_mode => return Some(Action::Exit),
            _ => {}
        }
        None
    }

    fn menu_key(&mut self, key: Key) -> Option<Action> {
        let Mode::Menu(menu) = &mut self.mode else { return None };
        match key {
            Key::Up if menu.focus > 0 => menu.focus -= 1,
            Key::Down if menu.focus + 1 < menu.items.len() => menu.focus += 1,
            Key::Back => self.mode = Mode::Browse,
            Key::Ok => {
                let item = menu.items[menu.focus];
                self.mode = Mode::Browse;
                return self.run(item);
            }
            _ => {}
        }
        None
    }

    fn run(&mut self, item: MenuItem) -> Option<Action> {
        match item {
            MenuItem::Move => {
                self.mode = Mode::Move { original: self.rails[FAVORITES].ids() };
                None
            }
            MenuItem::Hide => {
                let rail = &mut self.rails[FAVORITES];
                let mut tile = rail.tiles.remove(rail.focus);
                rail.focus = rail.focus.min(rail.tiles.len().saturating_sub(1));
                rail.reslot();
                self.prefs.hidden.push(tile.id.clone());
                let hidden = &mut self.rails[HIDDEN];
                tile.slot = Spring::new(hidden.tiles.len() as f32, 16.0);
                hidden.tiles.push(tile);
                if self.rails[FAVORITES].tiles.is_empty() {
                    self.active = HIDDEN;
                }
                self.save()
            }
            MenuItem::Show => {
                let rail = &mut self.rails[HIDDEN];
                let mut tile = rail.tiles.remove(rail.focus);
                rail.focus = rail.focus.min(rail.tiles.len().saturating_sub(1));
                rail.reslot();
                self.prefs.hidden.retain(|id| id != &tile.id);
                let favorites = &mut self.rails[FAVORITES];
                tile.slot = Spring::new(favorites.tiles.len() as f32, 16.0);
                favorites.tiles.push(tile);
                if self.rails[HIDDEN].tiles.is_empty() {
                    self.active = FAVORITES;
                    self.rails[FAVORITES].focus = self.rails[FAVORITES].tiles.len() - 1;
                }
                self.save()
            }
            MenuItem::Cancel => None,
        }
    }

    fn move_key(&mut self, key: Key) -> Option<Action> {
        let rail = &mut self.rails[FAVORITES];
        let focus = rail.focus;
        match key {
            Key::Left if focus > 0 => {
                rail.tiles.swap(focus, focus - 1);
                rail.focus -= 1;
                rail.reslot();
            }
            Key::Right if focus + 1 < rail.tiles.len() => {
                rail.tiles.swap(focus, focus + 1);
                rail.focus += 1;
                rail.reslot();
            }
            Key::Ok | Key::LongOk => {
                self.mode = Mode::Browse;
                return self.save();
            }
            Key::Back => {
                // Revert to the order from before the move.
                let Mode::Move { original } = std::mem::replace(&mut self.mode, Mode::Browse) else { return None };
                let rail = &mut self.rails[FAVORITES];
                let id = rail.tiles.get(focus).map(|t| t.id.clone());
                rail.tiles.sort_by_key(|t| original.iter().position(|o| o == &t.id).unwrap_or(usize::MAX));
                rail.focus = id.and_then(|id| rail.tiles.iter().position(|t| t.id == id)).unwrap_or(0);
                rail.reslot();
            }
            _ => {}
        }
        None
    }

    fn save(&mut self) -> Option<Action> {
        self.prefs.order = self.rails[FAVORITES].ids();
        Some(Action::SavePrefs(self.prefs.clone()))
    }

    fn focused(&self) -> Option<&Tile> {
        let rail = &self.rails[self.active];
        rail.tiles.get(rail.focus)
    }

    fn refocus(&mut self, instant: bool) {
        let moving = matches!(self.mode, Mode::Move { .. });
        let menu = matches!(self.mode, Mode::Menu(_));
        let active = self.active;
        for (r, rail) in self.rails.iter_mut().enumerate() {
            for (i, tile) in rail.tiles.iter_mut().enumerate() {
                tile.focus.target = if r == active && i == rail.focus { 1.0 } else { 0.0 };
                if instant {
                    tile.focus.snap();
                    tile.slot.snap();
                }
            }
            if !rail.tiles.is_empty() {
                rail.follow_focus();
            }
            if instant {
                rail.scroll.snap();
            }
        }
        self.overlay.target = if menu { 1.0 } else { 0.0 };
        self.lift.target = if moving { 1.0 } else { 0.0 };
        self.vscroll.target = -(self.rail_slot(active) as f32) * RAIL_PITCH;

        let s = self.strings;
        let next = self.focused().map(|tile| {
            let (sub, hint) = match (tile.kind, active) {
                _ if moving => (s.moving_hint.to_string(), None),
                (Kind::Input { connected: true }, _) => (s.connected_press_ok.to_string(), None),
                (Kind::Input { connected: false }, _) => (s.no_signal.to_string(), None),
                (Kind::App, HIDDEN) => (s.hidden_app.to_string(), None),
                (Kind::App, _) => (s.press_ok_to_open.to_string(), Some(s.hold_ok_for_options)),
            };
            Hero {
                eyebrow: if moving { s.moving.to_uppercase() } else { self.rail_title(active).to_uppercase() },
                title: tile.title.clone(),
                sub,
                hint,
            }
        });
        let changed = match (&self.hero, &next) {
            (Some(a), Some(b)) => a.title != b.title || a.eyebrow != b.eyebrow || a.sub != b.sub,
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
            self.vscroll.snap();
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
                moving |= tile.slot.step(dt);
            }
        }
        moving |= self.vscroll.step(dt);
        moving |= self.hero_t.step(dt);
        moving |= self.overlay.step(dt);
        moving |= self.lift.step(dt);
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
                tile.slot.snap();
            }
        }
        self.vscroll.snap();
        self.hero_t.snap();
        self.overlay.snap();
        self.lift.snap();
        self.previous_hero = None;
    }

    pub fn draw(&mut self, r: &mut Renderer, text: &mut Text, clock: &str, date: &str) {
        self.draw_backdrop(r);
        self.draw_header(r, text, clock, date);
        self.draw_hero(r, text);
        self.draw_rails(r, text);
        self.draw_menu(r, text);
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
        let accent = if self.lift.target > 0.0 { TEXT } else { self.ambient };
        if let Some(hero) = &self.previous_hero {
            if t < 1.0 {
                draw_hero_block(r, text, hero, 1.0 - t, -10.0 * t, accent);
            }
        }
        if let Some(hero) = &self.hero {
            draw_hero_block(r, text, hero, t, 14.0 * (1.0 - t), accent);
        }
    }

    fn draw_rails(&self, r: &mut Renderer, text: &mut Text) {
        let lift = self.lift.value.clamp(0.0, 1.0);
        // Three passes keep texture switches (shapes → icon atlas → glyph atlas) to a minimum.
        for pass in 0..3 {
            for (index, rail) in self.rails.iter().enumerate() {
                if rail.tiles.is_empty() {
                    continue;
                }
                let title_y = RAIL_TITLE_Y + self.rail_slot(index) as f32 * RAIL_PITCH + self.vscroll.value;
                let alpha = ((title_y - 380.0) / 240.0).clamp(0.0, 1.0);
                if alpha <= 0.0 || title_y > LOGICAL_H {
                    continue;
                }
                let tiles_y = title_y + 66.0;
                if pass == 2 {
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
                    let view = TileView { rect, f, alpha, lifted, icon: self.icons.get(&tile.id).copied() };
                    match pass {
                        0 => draw_tile_shapes(r, tile, &view),
                        1 => draw_tile_icon(r, self.icon_atlas, &view),
                        _ => draw_tile_text(r, text, tile, &view, self.strings),
                    }
                }
            }
        }
    }

    fn draw_menu(&self, r: &mut Renderer, text: &mut Text) {
        let open = self.overlay.value.clamp(0.0, 1.0);
        if open <= 0.01 {
            return;
        }
        r.rect(Rect::new(0.0, 0.0, LOGICAL_W, LOGICAL_H), 0.0, BG.alpha(0.6 * open));
        let Mode::Menu(menu) = &self.mode else { return };
        let Some(tile) = self.focused() else { return };
        let rail = &self.rails[self.active];
        let (row_h, pad, width) = (68.0, 16.0, 460.0);
        let height = 64.0 + pad + menu.items.len() as f32 * row_h + pad;
        let title_y = RAIL_TITLE_Y + self.rail_slot(self.active) as f32 * RAIL_PITCH + self.vscroll.value;
        let tile_x = rail.slot_x(rail.focus as f32) - rail.scroll.value;
        let x = tile_x.clamp(SAFE_X, LOGICAL_W - SAFE_X - width);
        let y = title_y - height - 24.0 + 16.0 * (1.0 - open);
        let panel = Rect::new(x, y, width, height);
        r.shadow(panel.offset(0.0, 24.0), 22.0, 60.0, Rgba::hex(0).alpha(0.5 * open));
        r.rect(panel, 22.0, Rgba::hex(0x14161C).alpha(0.96 * open));
        r.stroke(panel, 22.0, 1.0, TEXT.alpha(0.14 * open));
        let row = |i: usize| Rect::new(x + pad, y + 64.0 + pad + i as f32 * row_h, width - 2.0 * pad, row_h - 8.0);
        r.rect(row(menu.focus), 16.0, TEXT.alpha(open));
        text.draw_top(r, &tile.title, x + 28.0, y + 24.0, MENU_TITLE, TEXT.alpha(0.6 * open));
        for (i, item) in menu.items.iter().enumerate() {
            let label = match item {
                MenuItem::Move => self.strings.menu_move,
                MenuItem::Hide => self.strings.menu_hide,
                MenuItem::Show => self.strings.menu_show,
                MenuItem::Cancel => self.strings.menu_cancel,
            };
            let color = if i == menu.focus { BG } else { TEXT };
            let rect = row(i);
            text.draw(r, label, rect.x + 22.0, rect.y + 40.0, MENU_ITEM, color.alpha(open));
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

struct TileView {
    rect: Rect,
    f: f32,
    alpha: f32,
    lifted: f32,
    icon: Option<[f32; 5]>,
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
    if let Some(hint) = hero.hint {
        text.draw_top(r, hint, x, y + 50.0, HINT, TEXT.alpha(0.5 * alpha));
    }
}

fn draw_tile_shapes(r: &mut Renderer, tile: &Tile, v: &TileView) {
    let (rect, f, alpha) = (v.rect, v.f, v.alpha);
    if f > 0.0 {
        r.shadow(rect, TILE_RADIUS * 2.0, 80.0, tile.accent.alpha(0.45 * f * alpha));
        r.shadow(rect.offset(0.0, 30.0 + 16.0 * v.lifted), TILE_RADIUS, 60.0 + 30.0 * v.lifted, Rgba::hex(0).alpha(0.55 * f * alpha));
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

fn draw_tile_icon(r: &mut Renderer, atlas: Texture, v: &TileView) {
    let Some([u0, v0, u1, v1, aspect]) = v.icon else { return };
    let rect = v.rect;
    let side = rect.h * 0.58;
    let (w, h) = if aspect >= 1.0 { (side * aspect.min(1.8), side) } else { (side, side / aspect) };
    let icon = Rect::new(rect.x + (rect.w - w) * 0.5, rect.y + (rect.h - h) * 0.5, w, h);
    r.image(atlas, icon, [u0, v0, u1, v1], 18.0 * rect.h / 170.0, Rgba::hex(0xFFFFFF).alpha(v.alpha));
}

fn draw_tile_text(r: &mut Renderer, text: &mut Text, tile: &Tile, v: &TileView, s: &Strings) {
    let (rect, f, alpha) = (v.rect, v.f, v.alpha);
    if f > 0.0 {
        let ring = TEXT.lerp(tile.accent, v.lifted);
        r.stroke(rect.inflate(RING_OFFSET + RING_WIDTH * 0.5), TILE_RADIUS + RING_OFFSET, RING_WIDTH, ring.alpha(f * alpha));
    }
    match tile.kind {
        Kind::App => {
            if v.icon.is_none() {
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
            let sub = if connected { s.connected } else { s.no_signal };
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
