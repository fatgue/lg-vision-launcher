//! Home screen state, input and drawing (direction A, "Cinematic Ambient").
use crate::anim::Spring;
use crate::color::Rgba;
use crate::gl;
use crate::model::{
    version_newer, App, Entity, HaCategory, Icon, Input, InstallFailure, InstallState, Installed, MediaItem, Notice, Picture, Release, ServiceRequest,
    Status, TvFeed, Weather, TIVIONE_APP, TIVIONE_FEED_VERSION,
};
use crate::video::Stream;
use std::collections::HashSet;
use crate::render::{Rect, Renderer, Texture, LOGICAL_H, LOGICAL_W};
use crate::storage::Prefs;
use crate::strings::{self, Strings};
use crate::text::{Style, Text};
use crate::theme::*;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

const ICON_CELL: i32 = 128;
const ICON_ATLAS: i32 = 1024;
const ART_ATLAS: i32 = 2048;
const ART_CELL: (i32, i32) = (320, 180);
const RAIL_TITLE_Y: f32 = 620.0;
const RAIL_PITCH: f32 = 340.0;
const AMBIENT_SPEED: f32 = 6.0;

const FAVORITES: usize = 0;
const CONTINUE: usize = 1;
const TV_RECENT: usize = 2;
const TV_FAVORITES: usize = 3;
const TV_CATCHUP: usize = 4;
const TV_COMING: usize = 5;
const HOME: usize = 6;
const INPUTS: usize = 7;
const HIDDEN: usize = 8;
/// Where the live preview plays (16:9, right of the hero text).
pub const PREVIEW_RECT: Rect = Rect::new(1000.0, 150.0, 824.0, 464.0);
/// Focus must rest this long on a channel before its preview starts.
const PREVIEW_DWELL: Duration = Duration::from_millis(900);
/// No preview this long after another app started, so it never races that app for the decoder.
const PREVIEW_PAUSE: Duration = Duration::from_secs(5);
const DIGIT_WAIT: Duration = Duration::from_millis(1600);
const PLEX_APP: &str = "cdp-30";
/// TiviOne's store page: pictures live in one texture of 2×3 cells, 960×540 each.
const PICTURE_CELL: (i32, i32) = (960, 540);
const PICTURE_ATLAS: (i32, i32) = (1920, 1620);
const TIVIONE_TEAL: Rgba = Rgba::hex(0x2DD4BF);

pub enum Action {
    Launch { app_id: String, params: Option<Value> },
    SavePrefs(Prefs),
    Service(ServiceRequest),
    HomeTakeover(bool),
    RefreshImages,
    /// Read the app list again (after TiviOne was installed).
    Refresh,
    /// Download, verify and install TiviOne from its website.
    Install(Release),
    /// Load the pictures for TiviOne's store page.
    Pictures(Release),
    /// Start (Some) or stop (None) the live preview.
    Preview(Option<Stream>),
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
    Digit(u8),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Service {
    HomeAssistant,
    Plex,
    TiviOne,
    HomeTiles,
    Weather,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    App,
    Input { connected: bool },
    Media { progress: f32 },
    Entity { on: Option<bool> },
    Setting { service: Service, connected: bool },
    Channel { id: i64 },
    Reminder { id: i64, start: i64 },
    Catchup { id: i64, start: i64, end: i64 },
    /// TiviOne is not installed: opens its store page.
    Promo,
}

struct Tile {
    id: String,
    title: String,
    /// Media subtitle, entity state or setting status.
    detail: String,
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
    fn new(id: String, title: String, kind: Kind, index: usize) -> Self {
        Self {
            launch_id: id.clone(),
            id,
            title,
            detail: String::new(),
            params: None,
            kind,
            bg: TILE_NEUTRAL,
            accent: ACCENT_DEFAULT,
            focus: Spring::new(0.0, 18.0),
            slot: Spring::new(index as f32, 16.0),
        }
    }

    fn app(app: App, index: usize) -> Self {
        let mut tile = Self::new(app.id.clone(), app.title, Kind::App, index);
        tile.params = app.params;
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

    /// Replaces the tiles, keeping focus on the same id where possible.
    fn replace(&mut self, tiles: Vec<Tile>) {
        let focused = self.tiles.get(self.focus).map(|t| t.id.clone());
        self.tiles = tiles;
        self.focus = focused
            .and_then(|id| self.tiles.iter().position(|t| t.id == id))
            .unwrap_or(self.focus)
            .min(self.tiles.len().saturating_sub(1));
    }
}

#[derive(Clone, Copy, PartialEq)]
enum MenuItem {
    Move,
    Hide,
    Show,
    /// Hide the TiviOne tile (it stays in Settings).
    HidePromo,
    Cancel,
}

/// TiviOne's store page: banner, what it does, screenshots, install or update.
struct Store {
    /// 0: buttons, 1: screenshots.
    row: usize,
    button: usize,
    shot: usize,
    /// The focused screenshot fills the screen.
    full: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum StoreButton {
    Install,
    Update,
    Retry,
    Open,
    Hide,
    Close,
}

struct Menu {
    items: Vec<MenuItem>,
    focus: usize,
}

enum Mode {
    Browse,
    Menu(Menu),
    Move { original: Vec<String> },
    Setup { service: Service, closing_at: Option<Instant> },
    /// One-time TiviOne offer; `yes` = focus on "Turn on".
    Offer { yes: bool },
    /// Choosing the Home Assistant entities for the Home row.
    Picker(Picker),
    /// The Settings panel (opened from the gear in the header).
    Settings,
    /// TiviOne's store page.
    Store(Store),
}

const PICKER_ROWS: usize = 10;

/// Categories left, entities right; OK ticks, Back saves.
struct Picker {
    entities: Vec<Entity>,
    selected: HashSet<String>,
    loading: bool,
    /// 0: categories, 1: entities.
    column: usize,
    category: usize,
    row: usize,
    top: usize,
}

impl Picker {
    fn rows(&self) -> Vec<usize> {
        let category = HaCategory::ALL[self.category];
        self.entities.iter().enumerate().filter(|(_, e)| e.category == category).map(|(i, _)| i).collect()
    }

    fn count(&self, category: HaCategory) -> (usize, usize) {
        let all = self.entities.iter().filter(|e| e.category == category);
        let total = all.clone().count();
        (all.filter(|e| self.selected.contains(&e.id)).count(), total)
    }

    fn keep_visible(&mut self) {
        if self.row < self.top {
            self.top = self.row;
        } else if self.row >= self.top + PICKER_ROWS {
            self.top = self.row + 1 - PICKER_ROWS;
        }
    }

    /// The choice in display order (category, area, name).
    fn choice(&self) -> Vec<String> {
        self.entities.iter().filter(|e| self.selected.contains(&e.id)).map(|e| e.id.clone()).collect()
    }
}

/// Rows the user can switch off, with the rails they cover.
const ROW_KEYS: [(&str, &[usize]); 5] = [
    ("continue", &[CONTINUE]),
    ("tv", &[TV_RECENT, TV_FAVORITES, TV_CATCHUP, TV_COMING]),
    ("home", &[HOME]),
    ("inputs", &[INPUTS]),
    ("hidden", &[HIDDEN]),
];

#[derive(Clone, Copy, PartialEq, Debug)]
enum SettingId {
    Service(Service),
    WeatherShow,
    Location,
    Units,
    Clock24,
    ShowDate,
    HomeTakeover,
    Row(usize),
    Ambient,
    Motion,
    Preview,
    Language,
    RefreshImages,
    /// TiviOne's store page (install, update or about).
    TiviOneStore,
    /// The TiviOne tile on Home while TiviOne is missing.
    TiviOnePromo,
    Info,
}

struct SettingItem {
    id: SettingId,
    label: String,
    value: String,
    /// Switch state for on/off settings.
    on: Option<bool>,
    enabled: bool,
}

/// Something the Magic Remote pointer can hit, recorded while drawing.
#[derive(Clone, Copy, PartialEq)]
enum Target {
    Tile { rail: usize, index: usize },
    MenuItem(usize),
    Offer(bool),
    SetupButton,
    PickerCategory(usize),
    PickerRow(usize),
    Gear,
    SettingsSection(usize),
    SettingsRow(usize),
    StoreButton(usize),
    StoreShot(usize),
}

struct Hero {
    eyebrow: String,
    title: String,
    sub: String,
    hint: Option<&'static str>,
}

pub struct Home {
    strings: &'static Strings,
    rails: [Rail; 9],
    active: usize,
    mode: Mode,
    prefs: Prefs,
    apps: Vec<App>,
    status: Status,
    weather: Option<Weather>,
    tv: Option<TvFeed>,
    offered: bool,
    digits: Option<(String, Instant)>,
    pending: Vec<Action>,
    /// Channel whose live preview is playing, and where focus currently rests.
    preview: Option<i64>,
    dwell: Option<(i64, Instant)>,
    /// Set when another app starts: no preview until then (or until Vision is shown again).
    preview_paused_until: Option<Instant>,
    visible: bool,
    /// The TV's own UI language (read once).
    system_language: String,
    vscroll: Spring,
    overlay: Spring,
    lift: Spring,
    ambient: Rgba,
    hero: Option<Hero>,
    previous_hero: Option<Hero>,
    hero_t: Spring,
    icons: HashMap<String, [f32; 5]>,
    /// Icons whose opaque border colour fills their tile (drawn larger, seamless).
    blended: HashMap<String, Rgba>,
    arts: HashMap<String, (usize, [f32; 4])>,
    free_art: Vec<usize>,
    colors: HashMap<String, Rgba>,
    icon_atlas: Texture,
    icon_next: i32,
    art_atlas: Texture,
    art_next: usize,
    qr: Option<(String, qrcodegen::QrCode)>,
    notice: Option<(String, Instant)>,
    hits: RefCell<Vec<(Rect, Target)>>,
    /// Settings rows (connections, Home tiles, TiviOne), built from the service status.
    settings: Vec<Tile>,
    settings_section: usize,
    settings_column: usize,
    settings_row: usize,
    /// This process runs as LG's Home (Vision is the Home launcher).
    home_mode: bool,
    ip: Option<String>,
    /// The gear in the header has focus.
    header: bool,
    /// A setup panel or the picker was opened from Settings: Back returns there.
    from_settings: bool,
    /// TiviOne on this TV, the newest release on its website, and a running install.
    tivione: Option<Installed>,
    release: Option<Release>,
    install: InstallState,
    /// Store-page pictures, created on first use; which cells hold one.
    pictures: Option<Texture>,
    picture_loaded: [bool; crate::installer::MAX_PICTURES],
    pictures_requested: bool,
    pub loaded: bool,
}

impl Home {
    pub fn new(renderer: &Renderer, prefs: Prefs) -> Self {
        Self {
            strings: strings::current(),
            rails: [
                Rail::new(300.0, 170.0),
                Rail::new(300.0, 170.0),
                Rail::new(300.0, 170.0),
                Rail::new(300.0, 170.0),
                Rail::new(300.0, 150.0),
                Rail::new(300.0, 150.0),
                Rail::new(300.0, 150.0),
                Rail::new(300.0, 120.0),
                Rail::new(300.0, 170.0),
            ],
            active: FAVORITES,
            mode: Mode::Browse,
            prefs,
            apps: Vec::new(),
            status: Status::default(),
            weather: None,
            tv: None,
            offered: false,
            digits: None,
            pending: Vec::new(),
            preview: None,
            dwell: None,
            preview_paused_until: None,
            visible: true,
            system_language: crate::strings::system_language(),
            vscroll: Spring::new(0.0, 14.0),
            overlay: Spring::new(0.0, 22.0),
            lift: Spring::new(0.0, 18.0),
            ambient: ACCENT_DEFAULT,
            hero: None,
            previous_hero: None,
            hero_t: Spring::new(1.0, 14.0),
            icons: HashMap::new(),
            blended: HashMap::new(),
            arts: HashMap::new(),
            free_art: Vec::new(),
            colors: HashMap::new(),
            icon_atlas: renderer.create_texture(ICON_ATLAS, ICON_ATLAS, gl::RGBA),
            icon_next: 0,
            art_atlas: renderer.create_texture(ART_ATLAS, ART_ATLAS, gl::RGBA),
            art_next: 0,
            qr: None,
            notice: None,
            hits: RefCell::new(Vec::new()),
            settings: Vec::new(),
            settings_section: 0,
            settings_column: 0,
            settings_row: 0,
            home_mode: false,
            ip: None,
            header: false,
            from_settings: false,
            tivione: None,
            release: None,
            install: InstallState::Idle,
            pictures: None,
            picture_loaded: [false; crate::installer::MAX_PICTURES],
            pictures_requested: false,
            loaded: false,
        }
    }

    // ---- Magic Remote pointer ---------------------------------------------------------

    fn hit(&self, x: f32, y: f32) -> Option<Target> {
        // Last drawn wins: overlays are drawn after the rails.
        self.hits
            .borrow()
            .iter()
            .rev()
            .find(|(r, _)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
            .map(|(_, t)| *t)
    }

    /// Moves focus to whatever the pointer is over. Returns true when something changed.
    pub fn pointer(&mut self, x: f32, y: f32) -> bool {
        let Some(target) = self.hit(x, y) else { return false };
        let changed = match (target, &mut self.mode) {
            (Target::Gear, Mode::Browse) => !std::mem::replace(&mut self.header, true),
            (Target::SettingsSection(i), Mode::Settings) => {
                let changed = self.settings_section != i || self.settings_column != 0;
                if self.settings_section != i {
                    self.settings_row = 0;
                }
                self.settings_section = i;
                self.settings_column = 0;
                changed
            }
            (Target::SettingsRow(i), Mode::Settings) => {
                let changed = self.settings_row != i || self.settings_column != 1;
                self.settings_row = i;
                self.settings_column = 1;
                changed
            }
            (Target::Tile { rail, index }, Mode::Browse) => {
                let changed = self.active != rail || self.rails[rail].focus != index || self.header;
                self.header = false;
                self.active = rail;
                self.rails[rail].focus = index;
                changed
            }
            (Target::MenuItem(i), Mode::Menu(menu)) => std::mem::replace(&mut menu.focus, i) != i,
            (Target::Offer(yes), Mode::Offer { yes: current }) => std::mem::replace(current, yes) != yes,
            (Target::PickerCategory(i), Mode::Picker(p)) => {
                let changed = p.column != 0 || p.category != i;
                if p.category != i {
                    p.row = 0;
                    p.top = 0;
                }
                p.column = 0;
                p.category = i;
                changed
            }
            (Target::StoreButton(i), Mode::Store(store)) if !store.full => {
                let changed = store.row != 0 || store.button != i;
                store.row = 0;
                store.button = i;
                changed
            }
            (Target::StoreShot(i), Mode::Store(store)) if !store.full => {
                let changed = store.row != 1 || store.shot != i;
                store.row = 1;
                store.shot = i;
                changed
            }
            (Target::PickerRow(i), Mode::Picker(p)) => {
                let changed = p.column != 1 || p.row != i;
                p.column = 1;
                p.row = i;
                changed
            }
            _ => false,
        };
        if changed {
            self.refocus(false);
        }
        changed
    }

    /// Whether the pointer is over something a click activates (otherwise clicks are ignored).
    pub fn pointer_target(&self, x: f32, y: f32) -> bool {
        self.hit(x, y).is_some()
    }

    fn rail_title(&self, rail: usize) -> &'static str {
        let s = self.strings;
        match rail {
            FAVORITES => s.favorites,
            CONTINUE => s.continue_watching,
            TV_RECENT if self.tv.is_none() => s.live_tv,
            TV_RECENT => s.tv_recent,
            TV_FAVORITES => s.tv_favorites,
            TV_CATCHUP => s.catch_up,
            TV_COMING => s.tv_coming,
            HOME => s.home,
            INPUTS => s.inputs,
            _ => s.hidden,
        }
    }

    // ---- Data in --------------------------------------------------------------------

    pub fn set_apps(&mut self, apps: Vec<App>) {
        self.apps = apps;
        self.rebuild_app_rails();
        // The Settings row and the TiviOne offer depend on what is installed.
        self.set_status(self.status.clone());
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
        if matches!(self.mode, Mode::Menu(_) | Mode::Move { .. }) {
            self.mode = Mode::Browse;
        }
        // Vision always opens on the first favourite, whatever data arrived first.
        if !self.loaded {
            self.active = FAVORITES;
        }
        self.restore_focus(if self.loaded { focused_id } else { None });
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
        let tiles = inputs
            .into_iter()
            .enumerate()
            .map(|(i, input)| {
                let mut tile = Tile::new(input.id, input.label, Kind::Input { connected: input.connected }, i);
                tile.launch_id = input.app_id;
                tile.bg = Rgba::hex(0x14161C);
                tile.accent = ACCENT_INPUT;
                tile
            })
            .collect();
        self.rails[INPUTS].replace(tiles);
        self.refocus(true);
    }

    pub fn set_media(&mut self, items: Vec<MediaItem>) {
        let s = self.strings;
        let plex_app = self.apps.iter().find(|a| a.id == PLEX_APP || a.title == "Plex").map(|a| a.id.clone());
        let tiles = items
            .into_iter()
            .enumerate()
            .map(|(i, item)| {
                let mut tile = Tile::new(format!("plex:{}", item.id), item.title, Kind::Media { progress: item.progress }, i);
                let left = format!("{} {}", item.remaining_min, s.min_left);
                tile.detail = if item.subtitle.is_empty() { left } else { format!("{} · {left}", item.subtitle) };
                tile.launch_id = plex_app.clone().unwrap_or_else(|| PLEX_APP.into());
                if let Some(color) = self.colors.get(&tile.id) {
                    tile.tint(*color);
                }
                tile
            })
            .collect();
        self.rails[CONTINUE].replace(tiles);
        self.fix_active();
        self.refocus(false);
    }

    pub fn set_entities(&mut self, entities: Vec<Entity>) {
        let s = self.strings;
        let tiles = entities
            .into_iter()
            .enumerate()
            .map(|(i, entity)| {
                let state = entity_state(s, &entity);
                let detail = if entity.area.is_empty() { state } else { format!("{} · {state}", entity.area) };
                let mut tile = Tile::new(entity.id, entity.name, Kind::Entity { on: entity.on }, i);
                tile.detail = detail;
                tile.bg = Rgba::hex(0x15171D);
                tile.accent = if entity.on == Some(true) { Rgba::hex(0xF2C14E) } else { Rgba::hex(0x9AA3B2) };
                tile
            })
            .collect();
        self.rails[HOME].replace(tiles);
        self.fix_active();
        self.refocus(false);
    }

    pub fn set_status(&mut self, status: Status) {
        let s = self.strings;
        let setting = |service: Service, name: &str, account: &Option<String>, error: &Option<String>, i: usize| {
            let connected = account.is_some();
            let id = format!("setting:{service:?}");
            let mut tile = Tile::new(id, name.to_string(), Kind::Setting { service, connected }, i);
            tile.detail = match (account, error) {
                (Some(_), Some(error)) => error.clone(),
                (Some(_), None) => s.connected.to_string(),
                (None, _) => s.not_connected.to_string(),
            };
            tile.bg = Rgba::hex(0x14161C);
            tile.accent = Rgba::hex(0x9AA3B2);
            tile
        };
        let mut tiles = vec![
            setting(Service::HomeAssistant, s.home_assistant, &status.ha, &status.ha_error, 0),
            setting(Service::Plex, s.plex, &status.plex, &status.plex_error, 1),
        ];
        if status.ha.is_some() {
            let mut tile = Tile::new("setting:HomeTiles".into(), s.home_tiles.into(), Kind::Setting { service: Service::HomeTiles, connected: true }, tiles.len());
            tile.detail = if status.ha_selected == 0 { s.automatic.to_string() } else { format!("{} {}", status.ha_selected, s.selected) };
            tile.bg = Rgba::hex(0x14161C);
            tile.accent = Rgba::hex(0xF2C14E);
            tiles.insert(1, tile);
        }
        if self.apps.iter().any(|a| a.id == TIVIONE_APP) {
            let on = status.tivione == Some(true);
            let mut tile = Tile::new("setting:TiviOne".into(), s.tivione.into(), Kind::Setting { service: Service::TiviOne, connected: on }, 2);
            tile.detail = if on {
                s.shown_on_home
            } else if self.tivione_too_old() {
                s.tivione_needs_update
            } else {
                s.off
            }
            .to_string();
            tile.bg = Rgba::hex(0x14161C);
            tile.accent = Rgba::hex(0x2BB8A6);
            tiles.push(tile);
        }
        // Close a setup panel shortly after its service connects.
        if let Mode::Setup { service, closing_at } = &mut self.mode {
            let now_connected = match service {
                Service::HomeAssistant => status.ha.is_some() && self.status.ha.is_none(),
                Service::Plex => status.plex.is_some() && self.status.plex.is_none(),
                Service::Weather => status.location.is_some() && status.location != self.status.location,
                Service::TiviOne | Service::HomeTiles => false,
            };
            if now_connected {
                *closing_at = Some(Instant::now() + Duration::from_millis(1600));
            }
        }
        self.qr = status
            .ha_setup
            .as_ref()
            .or(status.location_setup.as_ref())
            .and_then(|url| qrcodegen::QrCode::encode_text(url, qrcodegen::QrCodeEcc::Medium).ok().map(|qr| (url.clone(), qr)));
        self.status = status;
        self.settings = tiles;
        self.maybe_offer();
        self.refocus(false);
    }

    /// Offers the TiviOne integration once, when TiviOne is installed and nothing was decided.
    fn maybe_offer(&mut self) {
        let installed = self.apps.iter().any(|a| a.id == TIVIONE_APP) && !self.tivione_too_old();
        if !self.offered && installed && self.status.tivione.is_none() && matches!(self.mode, Mode::Browse) && self.loaded {
            self.offered = true;
            self.mode = Mode::Offer { yes: true };
        }
    }

    pub fn set_tv(&mut self, feed: Option<TvFeed>) {
        self.tv = feed;
        let Some(tv) = &self.tv else {
            for rail in [TV_RECENT, TV_FAVORITES, TV_CATCHUP, TV_COMING] {
                self.rails[rail].replace(Vec::new());
            }
            self.update_promo();
            self.fix_active();
            self.refocus(false);
            return;
        };
        let channel_tiles = |ids: &[i64]| -> Vec<Tile> {
            ids.iter()
                .filter_map(|id| tv.channels.get(id).map(|c| (*id, c)))
                .enumerate()
                .map(|(i, (id, channel))| {
                    let mut tile = Tile::new(format!("tv:{id}"), channel.name.clone(), Kind::Channel { id }, i);
                    tile.detail = channel.number.map(|n| n.to_string()).unwrap_or_default();
                    tile.launch_id = TIVIONE_APP.into();
                    tile.params = Some(serde_json::json!({ "channelId": id, "playlistId": tv.playlist_id }));
                    tile.bg = Rgba::hex(0x0E1A2E);
                    tile.accent = Rgba::hex(0x3E7BFA);
                    tile
                })
                .collect()
        };
        let recent = channel_tiles(&tv.recent);
        let favorites = channel_tiles(&tv.favorites);
        let coming: Vec<Tile> = tv
            .reminders
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let mut tile = Tile::new(format!("reminder:{}:{}", r.channel, r.start), r.title.clone(), Kind::Reminder { id: r.channel, start: r.start }, i);
                tile.detail = tv.channels.get(&r.channel).map(|c| c.name.clone()).unwrap_or_default();
                tile.launch_id = TIVIONE_APP.into();
                tile.params = Some(serde_json::json!({ "channelId": r.channel, "playlistId": tv.playlist_id }));
                tile.bg = Rgba::hex(0x15171D);
                tile.accent = Rgba::hex(0xF2C14E);
                tile
            })
            .collect();
        let catchup: Vec<Tile> = tv
            .catchup
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let mut tile = Tile::new(format!("catchup:{}:{}", c.channel, c.start), c.title.clone(), Kind::Catchup { id: c.channel, start: c.start, end: c.end }, i);
                tile.detail = tv.channels.get(&c.channel).map(|ch| ch.name.clone()).unwrap_or_default();
                tile.launch_id = TIVIONE_APP.into();
                tile.params = Some(serde_json::json!({
                    "catchup": { "channelId": c.channel, "startMs": c.start, "stopMs": c.end, "title": c.title },
                    "playlistId": tv.playlist_id,
                }));
                tile.bg = Rgba::hex(0x161A24);
                tile.accent = Rgba::hex(0x7FB2FF);
                tile
            })
            .collect();
        self.rails[TV_RECENT].replace(recent);
        self.rails[TV_FAVORITES].replace(favorites);
        self.rails[TV_CATCHUP].replace(catchup);
        self.rails[TV_COMING].replace(coming);
        self.fix_active();
        self.refocus(false);
    }

    // ---- TiviOne: store page, install and update -----------------------------------

    pub fn set_tivione(&mut self, installed: Option<Installed>) {
        if self.tivione == installed {
            return;
        }
        self.tivione = installed;
        self.update_promo();
        self.set_status(self.status.clone());
    }

    pub fn set_release(&mut self, release: Option<Release>) {
        if self.release == release {
            return;
        }
        if self.release.as_ref().map(|r| &r.pictures) != release.as_ref().map(|r| &r.pictures) {
            self.pictures_requested = false;
        }
        self.release = release;
        self.update_promo();
        self.refocus(false);
    }

    pub fn set_install(&mut self, state: InstallState) {
        if state == InstallState::Done {
            self.install = InstallState::Idle;
            self.notice = Some((self.strings.tivione_installed.to_string(), Instant::now()));
            if matches!(self.mode, Mode::Store(_)) {
                self.mode = self.after_overlay();
            }
            // The new app list brings the TiviOne tile and the offer to connect it.
            self.offered = false;
            self.pending.push(Action::Refresh);
        } else {
            self.install = state;
        }
        self.refocus(false);
    }

    pub fn add_picture(&mut self, picture: Picture, renderer: &Renderer) {
        if picture.index >= self.picture_loaded.len() {
            return;
        }
        let texture = *self.pictures.get_or_insert_with(|| renderer.create_texture(PICTURE_ATLAS.0, PICTURE_ATLAS.1, gl::RGBA));
        let (x, y) = picture_cell(picture.index);
        renderer.upload(texture, x, y, picture.w as i32, picture.h as i32, gl::RGBA, &picture.rgba);
        self.picture_loaded[picture.index] = true;
    }

    /// Texture and UV rectangle of a loaded store picture (0: banner, then screenshots).
    fn picture(&self, index: usize) -> Option<(Texture, [f32; 4])> {
        let texture = self.pictures?;
        if !self.picture_loaded.get(index).copied().unwrap_or(false) {
            return None;
        }
        let (x, y) = picture_cell(index);
        let (w, h) = (PICTURE_ATLAS.0 as f32, PICTURE_ATLAS.1 as f32);
        Some((texture, [x as f32 / w, y as f32 / h, (x + PICTURE_CELL.0) as f32 / w, (y + PICTURE_CELL.1) as f32 / h]))
    }

    /// Installed, but a version from before the launcher feed (it can only be opened).
    fn tivione_too_old(&self) -> bool {
        self.tivione.as_ref().is_some_and(|i| version_newer(TIVIONE_FEED_VERSION, &i.version))
    }

    /// A newer release for a TiviOne that Vision installed (store installs update via LG).
    fn update_available(&self) -> bool {
        matches!((&self.tivione, &self.release), (Some(i), Some(r)) if i.direct && version_newer(&r.version, &i.version))
    }

    /// The TiviOne tile on Home: while TiviOne is missing, a release is available and the
    /// user has not hidden it.
    fn promo_visible(&self) -> bool {
        self.tv.is_none()
            && self.release.is_some()
            && self.prefs.ui.tivione_promo
            && self.tivione.is_none()
            && !self.apps.iter().any(|a| a.id == TIVIONE_APP)
    }

    fn update_promo(&mut self) {
        if self.tv.is_some() {
            return;
        }
        let shown = self.rails[TV_RECENT].tiles.first().is_some_and(|t| t.kind == Kind::Promo);
        let visible = self.promo_visible();
        if shown == visible {
            return;
        }
        let tiles = if visible {
            let mut tile = Tile::new("promo:tivione".into(), "TiviOne".into(), Kind::Promo, 0);
            tile.detail = self.strings.promo_detail.to_string();
            tile.bg = Rgba::hex(0x0A3236);
            tile.accent = TIVIONE_TEAL;
            vec![tile]
        } else {
            Vec::new()
        };
        self.rails[TV_RECENT].replace(tiles);
        self.fix_active();
        self.refocus(false);
    }

    fn open_store(&mut self) -> Option<Action> {
        self.mode = Mode::Store(Store { row: 0, button: 0, shot: 0, full: false });
        let release = self.release.as_ref().filter(|_| !self.pictures_requested)?.clone();
        self.pictures_requested = true;
        Some(Action::Pictures(release))
    }

    fn store_buttons(&self) -> Vec<StoreButton> {
        let mut buttons = Vec::new();
        let busy = matches!(self.install, InstallState::Downloading(_) | InstallState::Installing);
        let failed = matches!(self.install, InstallState::Failed(_));
        if !busy {
            match (&self.tivione, &self.release) {
                (None, Some(_)) => buttons.push(if failed { StoreButton::Retry } else { StoreButton::Install }),
                (Some(_), _) if self.update_available() => buttons.push(if failed { StoreButton::Retry } else { StoreButton::Update }),
                (Some(_), _) => buttons.push(StoreButton::Open),
                (None, None) => {}
            }
        }
        buttons.push(if self.tivione.is_none() && self.prefs.ui.tivione_promo { StoreButton::Hide } else { StoreButton::Close });
        buttons
    }

    fn store_key(&mut self, key: Key) -> Option<Action> {
        let buttons = self.store_buttons();
        let shots = self.release.as_ref().map_or(0, |r| r.pictures.len().saturating_sub(1));
        let Mode::Store(store) = &mut self.mode else { return None };
        store.button = store.button.min(buttons.len() - 1);
        if store.full {
            match key {
                Key::Left if store.shot > 0 => store.shot -= 1,
                Key::Right if store.shot + 1 < shots => store.shot += 1,
                Key::Ok | Key::Back => store.full = false,
                _ => {}
            }
            return None;
        }
        match (key, store.row) {
            (Key::Back, _) => self.mode = self.after_overlay(),
            (Key::Up, _) => store.row = 0,
            (Key::Down, _) if shots > 0 => store.row = 1,
            (Key::Left, 0) if store.button > 0 => store.button -= 1,
            (Key::Right, 0) if store.button + 1 < buttons.len() => store.button += 1,
            (Key::Left, 1) if store.shot > 0 => store.shot -= 1,
            (Key::Right, 1) if store.shot + 1 < shots => store.shot += 1,
            (Key::Ok, 1) => store.full = true,
            (Key::Ok, _) => {
                let button = buttons[store.button];
                return self.store_press(button);
            }
            _ => {}
        }
        None
    }

    fn store_press(&mut self, button: StoreButton) -> Option<Action> {
        match button {
            StoreButton::Install | StoreButton::Update | StoreButton::Retry => {
                let release = self.release.clone()?;
                self.install = InstallState::Downloading(0.0);
                Some(Action::Install(release))
            }
            StoreButton::Open => {
                self.mode = Mode::Browse;
                self.from_settings = false;
                Some(Action::Launch { app_id: TIVIONE_APP.into(), params: None })
            }
            StoreButton::Hide => {
                self.mode = self.after_overlay();
                self.hide_promo()
            }
            StoreButton::Close => {
                self.mode = self.after_overlay();
                None
            }
        }
    }

    fn hide_promo(&mut self) -> Option<Action> {
        self.prefs.ui.tivione_promo = false;
        self.update_promo();
        self.notice = Some((self.strings.promo_hidden.to_string(), Instant::now()));
        Some(Action::SavePrefs(self.prefs.clone()))
    }

    /// Called every minute: the guide moves on, so the hero text may change.
    pub fn tick(&mut self) {
        if self.tv.is_some() {
            self.refocus(false);
        }
    }

    /// An action produced by a timer (digit entry), picked up by the event loop.
    pub fn take_action(&mut self) -> Option<Action> {
        (!self.pending.is_empty()).then(|| self.pending.remove(0))
    }

    /// Another app is starting: the caller releases the decoder; no preview for a few seconds,
    /// or until Vision is shown again. Returns whether a preview was playing.
    pub fn pause_preview(&mut self) -> bool {
        self.dwell = None;
        self.preview_paused_until = Some(Instant::now() + PREVIEW_PAUSE);
        self.preview.take().is_some()
    }

    /// Visibility of Vision's window: a hidden Home stops its preview at once.
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
        if visible {
            self.preview_paused_until = None;
        } else {
            self.dwell = None;
            if self.preview.take().is_some() {
                self.pending.push(Action::Preview(None));
            }
        }
    }

    /// Starts the preview once focus rests on a channel with a stream; stops it otherwise.
    fn update_preview(&mut self) {
        let wanted = match (self.focused().map(|t| t.kind), &self.tv) {
            (Some(Kind::Channel { id }), Some(tv))
                if self.visible
                    && self.prefs.ui.preview
                    && self.preview_paused_until.is_none_or(|until| Instant::now() >= until)
                    && matches!(self.mode, Mode::Browse)
                    && !self.header
                    && tv.streams.contains_key(&id) =>
            {
                Some(id)
            }
            _ => None,
        };
        match wanted {
            Some(id) if self.preview == Some(id) => {}
            Some(id) => {
                if self.preview.take().is_some() {
                    self.pending.push(Action::Preview(None));
                }
                match self.dwell {
                    Some((d, since)) if d == id => {
                        if since.elapsed() >= PREVIEW_DWELL {
                            let stream = self.tv.as_ref().and_then(|tv| tv.streams.get(&id)).cloned();
                            self.pending.push(Action::Preview(stream));
                            self.preview = Some(id);
                        }
                    }
                    _ => self.dwell = Some((id, Instant::now())),
                }
            }
            None => {
                self.dwell = None;
                if self.preview.take().is_some() {
                    self.pending.push(Action::Preview(None));
                }
            }
        }
    }

    /// Whether a timer (preview dwell) needs another frame soon.
    pub fn waiting(&self) -> bool {
        self.dwell.is_some_and(|(id, _)| self.preview != Some(id))
    }

    pub fn set_catalog(&mut self, entities: Vec<Entity>, selected: Vec<String>) {
        if let Mode::Picker(picker) = &mut self.mode {
            picker.entities = entities;
            picker.selected = selected.into_iter().collect();
            picker.loading = false;
            // Open on the first category that has something.
            picker.category = HaCategory::ALL.iter().position(|c| picker.count(*c).1 > 0).unwrap_or(0);
        }
    }

    pub fn set_weather(&mut self, weather: Option<Weather>) {
        self.weather = weather;
    }

    fn fix_active(&mut self) {
        if self.rails[self.active].tiles.is_empty() {
            self.active = (0..self.rails.len()).find(|&r| self.has(r)).unwrap_or(FAVORITES);
        }
    }

    fn tint_id(&mut self, id: &str, color: Rgba) {
        self.colors.insert(id.to_string(), color);
        for rail in self.rails.iter_mut() {
            for tile in rail.tiles.iter_mut().filter(|t| t.id == id) {
                tile.tint(color);
            }
        }
    }

    /// Full-bleed tile artwork (320×180) for an app or a Plex item.
    pub fn add_art(&mut self, art: Icon, renderer: &Renderer) {
        if let Some(color) = art.color {
            self.tint_id(&art.app_id, Rgba::from_u8(color));
        }
        if self.arts.contains_key(&art.app_id) {
            return;
        }
        let Some(slot) = self.alloc_art_slot() else { return };
        let cols = (ART_ATLAS / ART_CELL.0) as usize;
        let (x, y) = ((slot % cols) as i32 * ART_CELL.0, (slot / cols) as i32 * ART_CELL.1);
        renderer.upload(self.art_atlas, x, y, art.w as i32, art.h as i32, gl::RGBA, &art.pixels);
        let a = ART_ATLAS as f32;
        // Half-texel inset keeps linear filtering from bleeding in the neighbouring cell.
        let uv = [(x as f32 + 0.5) / a, (y as f32 + 0.5) / a, (x as f32 + art.w as f32 - 0.5) / a, (y as f32 + art.h as f32 - 0.5) / a];
        self.arts.insert(art.app_id, (slot, uv));
    }

    /// Next free artwork cell; when full, Plex artwork no longer on screen is recycled.
    fn alloc_art_slot(&mut self) -> Option<usize> {
        let capacity = ((ART_ATLAS / ART_CELL.0) * (ART_ATLAS / ART_CELL.1)) as usize;
        if let Some(slot) = self.free_art.pop() {
            return Some(slot);
        }
        if self.art_next < capacity {
            self.art_next += 1;
            return Some(self.art_next - 1);
        }
        let live: Vec<String> = self.rails[CONTINUE].ids();
        let stale: Vec<String> = self.arts.keys().filter(|id| id.starts_with("plex:") && !live.contains(id)).cloned().collect();
        for id in stale {
            if let Some((slot, _)) = self.arts.remove(&id) {
                self.free_art.push(slot);
            }
        }
        self.free_art.pop()
    }

    /// Packs an icon into the shared atlas once and tints every tile of that app.
    pub fn add_icon(&mut self, icon: Icon, renderer: &Renderer) {
        if let Some(color) = icon.color {
            if !self.arts.contains_key(&icon.app_id) {
                self.tint_id(&icon.app_id, Rgba::from_u8(color));
            }
        }
        if let Some(edge) = icon.edge_color() {
            self.blended.insert(icon.app_id.clone(), Rgba::from_u8(edge));
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
        self.rails.get(rail).is_some_and(|r| !r.tiles.is_empty()) && !self.row_off(rail)
    }

    /// Whether the user switched this rail's row off in Settings.
    fn row_off(&self, rail: usize) -> bool {
        ROW_KEYS.iter().any(|(key, rails)| rails.contains(&rail) && self.prefs.ui.hidden_rows.iter().any(|h| h == key))
    }

    pub fn set_home_mode(&mut self, home_mode: bool) {
        self.home_mode = home_mode;
    }

    /// Vertical slot of a rail among the non-empty ones.
    fn rail_slot(&self, rail: usize) -> usize {
        (0..rail).filter(|&r| self.has(r)).count()
    }

    // ---- Input ----------------------------------------------------------------------

    pub fn key(&mut self, key: Key, home_mode: bool) -> Option<Action> {
        let action = match &self.mode {
            Mode::Menu(_) => self.menu_key(key),
            Mode::Move { .. } => self.move_key(key),
            Mode::Setup { .. } => self.setup_key(key),
            Mode::Offer { .. } => self.offer_key(key),
            Mode::Picker(_) => self.picker_key(key),
            Mode::Settings => self.settings_key(key),
            Mode::Store(_) => self.store_key(key),
            Mode::Browse if self.header => self.header_key(key),
            Mode::Browse if matches!(key, Key::Digit(_)) || (self.digits.is_some() && key == Key::Ok) => self.digit_key(key),
            Mode::Browse => self.browse_key(key, home_mode),
        };
        self.refocus(false);
        action
    }

    /// Where an overlay opened from Settings returns to.
    fn after_overlay(&mut self) -> Mode {
        if std::mem::take(&mut self.from_settings) {
            Mode::Settings
        } else {
            Mode::Browse
        }
    }

    fn header_key(&mut self, key: Key) -> Option<Action> {
        match key {
            Key::Down | Key::Back => self.header = false,
            Key::Ok => {
                self.mode = Mode::Settings;
                self.settings_column = 0;
                self.ip = crate::net::local_ip();
            }
            _ => {}
        }
        None
    }

    /// Everything the Settings panel lists, by section.
    fn settings_sections(&self) -> Vec<(&'static str, Vec<SettingItem>)> {
        let s = self.strings;
        let ui = &self.prefs.ui;
        let onoff = |v: bool| if v { s.on } else { s.off }.to_string();
        let item = |id, label: &str, value: String, on: Option<bool>| SettingItem { id, label: label.to_string(), value, on, enabled: true };
        let mut connections: Vec<SettingItem> = self
            .settings
            .iter()
            .filter_map(|tile| match tile.kind {
                Kind::Setting { service, connected } => Some(item(SettingId::Service(service), &tile.title, tile.detail.clone(), (service == Service::TiviOne).then_some(connected))),
                _ => None,
            })
            .collect();
        // TiviOne always has an entry: get it, update it, or read about it.
        let (label, value) = match (&self.tivione, &self.release) {
            (None, Some(r)) => (s.tivione_get, r.version.clone()),
            (None, None) => (s.tivione_get, s.store_unavailable.to_string()),
            (Some(_), Some(r)) if self.update_available() => (s.tivione_update, r.version.clone()),
            (Some(i), _) => (s.tivione_about, i.version.clone()),
        };
        connections.push(item(SettingId::TiviOneStore, label, value, None));
        if self.tivione.is_none() {
            connections.push(item(SettingId::TiviOnePromo, s.tivione_on_home, onoff(ui.tivione_promo), Some(ui.tivione_promo)));
        }
        let location = match (&self.status.location, &self.status.ha) {
            (Some(name), _) => name.clone(),
            (None, Some(_)) => s.from_home_assistant.to_string(),
            (None, None) => s.not_set.to_string(),
        };
        let weather = vec![
            item(SettingId::WeatherShow, s.show_weather, onoff(ui.weather), Some(ui.weather)),
            item(SettingId::Location, s.location, location, None),
            item(SettingId::Units, s.units, if ui.fahrenheit { s.fahrenheit } else { s.celsius }.to_string(), None),
        ];
        let clock = vec![
            item(SettingId::Clock24, s.time_format, if ui.clock24 { s.hours24 } else { s.hours12 }.to_string(), None),
            item(SettingId::ShowDate, s.show_date, onoff(ui.show_date), Some(ui.show_date)),
        ];
        let home = vec![item(SettingId::HomeTakeover, s.vision_as_home, onoff(self.home_mode), Some(self.home_mode))];
        let rows = ROW_KEYS
            .iter()
            .enumerate()
            .map(|(i, (key, _))| {
                let label = [s.continue_watching, s.live_tv, s.home, s.inputs, s.hidden][i];
                let shown = !ui.hidden_rows.iter().any(|h| h == key);
                item(SettingId::Row(i), label, if shown { s.shown } else { s.row_hidden }.to_string(), Some(shown))
            })
            .collect();
        let appearance = vec![
            item(SettingId::Ambient, s.ambient_light, s.levels[ui.ambient.min(3) as usize].to_string(), None),
            item(SettingId::Motion, s.motion, if ui.reduced_motion { s.reduced } else { s.full }.to_string(), None),
            item(SettingId::Preview, s.live_preview, onoff(ui.preview), Some(ui.preview)),
        ];
        let language = match crate::strings::LANGUAGES.iter().find(|(code, _, _)| *code == ui.language) {
            Some((_, name, _)) => name.to_string(),
            None => {
                let system = &self.system_language;
                let name = crate::strings::LANGUAGES.iter().find(|(c, _, _)| system.starts_with(c)).map_or("English", |(_, n, _)| n);
                format!("{} ({name})", s.automatic)
            }
        };
        let mut system = vec![
            item(SettingId::Language, s.language, language, None),
            item(SettingId::RefreshImages, s.refresh_images, String::new(), None),
        ];
        let info = |label: &str, value: String| SettingItem { id: SettingId::Info, label: label.to_string(), value, on: None, enabled: false };
        system.push(info(s.launcher, if self.home_mode { "Vision" } else { s.lg_home }.to_string()));
        system.push(info(s.ip_address, self.ip.clone().unwrap_or_else(|| "–".into())));
        system.push(info(s.version, env!("CARGO_PKG_VERSION").to_string()));
        let names = s.sections;
        vec![
            (names[0], connections),
            (names[1], weather),
            (names[2], clock),
            (names[3], home),
            (names[4], rows),
            (names[5], appearance),
            (names[6], system),
        ]
    }

    fn settings_key(&mut self, key: Key) -> Option<Action> {
        let sections = self.settings_sections();
        let count = sections.get(self.settings_section).map_or(0, |(_, items)| items.len());
        match (key, self.settings_column) {
            (Key::Back, 0) => {
                self.mode = Mode::Browse;
                self.header = true;
            }
            (Key::Back | Key::Left, 1) => self.settings_column = 0,
            (Key::Up, 0) if self.settings_section > 0 => {
                self.settings_section -= 1;
                self.settings_row = 0;
            }
            (Key::Down, 0) if self.settings_section + 1 < sections.len() => {
                self.settings_section += 1;
                self.settings_row = 0;
            }
            (Key::Right | Key::Ok, 0) if count > 0 => self.settings_column = 1,
            (Key::Up, 1) if self.settings_row > 0 => self.settings_row -= 1,
            (Key::Down, 1) if self.settings_row + 1 < count => self.settings_row += 1,
            (Key::Ok, 1) => {
                let item = sections.into_iter().nth(self.settings_section)?.1.into_iter().nth(self.settings_row)?;
                if !item.enabled {
                    return None;
                }
                return self.change_setting(item.id);
            }
            _ => {}
        }
        None
    }

    fn change_setting(&mut self, id: SettingId) -> Option<Action> {
        let ui = &mut self.prefs.ui;
        match id {
            SettingId::Service(service) => {
                let connected = self.settings.iter().find_map(|t| match t.kind {
                    Kind::Setting { service: s, connected } if s == service => Some(connected),
                    _ => None,
                })?;
                let action = self.open_setting(service, connected);
                if !matches!(self.mode, Mode::Settings) {
                    self.from_settings = true;
                }
                return action;
            }
            SettingId::Location => {
                self.mode = Mode::Setup { service: Service::Weather, closing_at: None };
                self.from_settings = true;
                return Some(Action::Service(ServiceRequest::LocationSetup(true)));
            }
            SettingId::HomeTakeover => {
                self.notice = Some((self.strings.switching.to_string(), Instant::now()));
                return Some(Action::HomeTakeover(!self.home_mode));
            }
            SettingId::RefreshImages => {
                self.clear_images();
                self.notice = Some((self.strings.reloading_images.to_string(), Instant::now()));
                return Some(Action::RefreshImages);
            }
            SettingId::Info => return None,
            SettingId::TiviOneStore => {
                let action = self.open_store();
                self.from_settings = true;
                return action;
            }
            SettingId::TiviOnePromo => ui.tivione_promo = !ui.tivione_promo,
            SettingId::WeatherShow => ui.weather = !ui.weather,
            SettingId::Units => ui.fahrenheit = !ui.fahrenheit,
            SettingId::Clock24 => ui.clock24 = !ui.clock24,
            SettingId::ShowDate => ui.show_date = !ui.show_date,
            SettingId::Ambient => ui.ambient = (ui.ambient + 1) % 4,
            SettingId::Motion => ui.reduced_motion = !ui.reduced_motion,
            SettingId::Preview => ui.preview = !ui.preview,
            SettingId::Language => {
                // Automatic → English → Deutsch → … → Automatic.
                let codes: Vec<&str> = std::iter::once("").chain(crate::strings::LANGUAGES.iter().map(|(c, _, _)| *c)).collect();
                let at = codes.iter().position(|c| *c == ui.language).unwrap_or(0);
                ui.language = codes[(at + 1) % codes.len()].to_string();
                let code = if ui.language.is_empty() { self.system_language.clone() } else { ui.language.clone() };
                crate::strings::set(&code);
                self.strings = crate::strings::current();
                self.hero = None;
                self.refocus(true);
            }
            SettingId::Row(i) => {
                let key = ROW_KEYS[i].0.to_string();
                if let Some(at) = ui.hidden_rows.iter().position(|h| *h == key) {
                    ui.hidden_rows.remove(at);
                } else {
                    ui.hidden_rows.push(key);
                }
                self.fix_active();
            }
        }
        self.update_promo();
        Some(Action::SavePrefs(self.prefs.clone()))
    }

    /// Forgets uploaded icons and artwork; the platform worker sends them again.
    fn clear_images(&mut self) {
        self.icons.clear();
        self.blended.clear();
        self.arts.clear();
        self.free_art.clear();
        self.icon_next = 0;
        self.art_next = 0;
    }

    fn browse_key(&mut self, key: Key, home_mode: bool) -> Option<Action> {
        let active = self.active;
        let (focus, count) = (self.rails[active].focus, self.rails[active].tiles.len());
        match key {
            Key::Left if focus > 0 => self.rails[active].focus -= 1,
            Key::Right if focus + 1 < count => self.rails[active].focus += 1,
            Key::Up => match (0..active).rev().find(|&r| self.has(r)) {
                Some(rail) => self.active = rail,
                // Above the top row: the Settings gear.
                None => self.header = true,
            },
            Key::Down => {
                if let Some(rail) = (active + 1..self.rails.len()).find(|&r| self.has(r)) {
                    self.active = rail;
                }
            }
            Key::Ok => return self.activate(),
            Key::LongOk if active == FAVORITES && count > 0 => {
                self.mode = Mode::Menu(Menu { items: vec![MenuItem::Move, MenuItem::Hide, MenuItem::Cancel], focus: 0 });
            }
            Key::LongOk if self.focused().is_some_and(|t| t.kind == Kind::Promo) => {
                self.mode = Mode::Menu(Menu { items: vec![MenuItem::HidePromo, MenuItem::Cancel], focus: 0 });
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

    fn picker_key(&mut self, key: Key) -> Option<Action> {
        let Mode::Picker(p) = &mut self.mode else { return None };
        if key == Key::Back {
            let choice = p.choice();
            let loaded = !p.loading;
            self.mode = self.after_overlay();
            return loaded.then_some(Action::Service(ServiceRequest::HaSelect(choice)));
        }
        if p.loading {
            return None;
        }
        let rows = p.rows();
        match (key, p.column) {
            (Key::Up, 0) if p.category > 0 => {
                p.category -= 1;
                p.row = 0;
                p.top = 0;
            }
            (Key::Down, 0) if p.category + 1 < HaCategory::ALL.len() => {
                p.category += 1;
                p.row = 0;
                p.top = 0;
            }
            (Key::Right | Key::Ok, 0) if !rows.is_empty() => p.column = 1,
            (Key::Left, 1) => p.column = 0,
            (Key::Up, 1) if p.row > 0 => p.row -= 1,
            (Key::Down, 1) if p.row + 1 < rows.len() => p.row += 1,
            (Key::Ok, 1) => {
                if let Some(&index) = rows.get(p.row) {
                    let id = p.entities[index].id.clone();
                    if !p.selected.remove(&id) {
                        p.selected.insert(id);
                    }
                }
            }
            _ => {}
        }
        p.keep_visible();
        None
    }

    fn offer_key(&mut self, key: Key) -> Option<Action> {
        let Mode::Offer { yes } = &mut self.mode else { return None };
        match key {
            Key::Left | Key::Right => *yes = !*yes,
            Key::Ok => {
                let choice = *yes;
                self.mode = Mode::Browse;
                return Some(Action::Service(ServiceRequest::TiviOne(choice)));
            }
            Key::Back => {
                self.mode = Mode::Browse;
                return Some(Action::Service(ServiceRequest::TiviOne(false)));
            }
            _ => {}
        }
        None
    }

    /// Direct channel entry with the digit keys (TiviOne on): tunes after a short pause or OK.
    fn digit_key(&mut self, key: Key) -> Option<Action> {
        if self.tv.is_none() {
            return None;
        }
        match key {
            Key::Digit(d) => {
                let entry = self.digits.get_or_insert((String::new(), Instant::now()));
                if entry.0.len() < 4 {
                    entry.0.push((b'0' + d) as char);
                }
                entry.1 = Instant::now();
                None
            }
            _ => self.tune_digits(),
        }
    }

    fn tune_digits(&mut self) -> Option<Action> {
        let (entry, _) = self.digits.take()?;
        let tv = self.tv.as_ref()?;
        let number: u32 = entry.parse().ok()?;
        match tv.numbers.get(&number) {
            Some(id) => Some(Action::Launch {
                app_id: TIVIONE_APP.into(),
                params: Some(serde_json::json!({ "channelId": id, "playlistId": tv.playlist_id })),
            }),
            None => {
                self.notice = Some((format!("{} {number}", self.strings.no_channel), Instant::now()));
                None
            }
        }
    }

    fn activate(&mut self) -> Option<Action> {
        let rail = &mut self.rails[self.active];
        let tile = rail.tiles.get_mut(rail.focus)?;
        match tile.kind {
            Kind::App | Kind::Input { .. } | Kind::Media { .. } | Kind::Channel { .. } | Kind::Reminder { .. } | Kind::Catchup { .. } => {
                Some(Action::Launch { app_id: tile.launch_id.clone(), params: tile.params.clone() })
            }
            Kind::Entity { on } => {
                // Optimistic: lights flip at once; the next refresh confirms.
                if let Some(on) = on {
                    tile.kind = Kind::Entity { on: Some(!on) };
                    tile.detail = if on { self.strings.off } else { self.strings.on }.to_string();
                    tile.accent = if on { Rgba::hex(0x9AA3B2) } else { Rgba::hex(0xF2C14E) };
                }
                Some(Action::Service(ServiceRequest::HaAction(tile.id.clone())))
            }
            Kind::Setting { service, connected } => self.open_setting(service, connected),
            Kind::Promo => self.open_store(),
        }
    }

    fn open_setting(&mut self, service: Service, connected: bool) -> Option<Action> {
        match (service, connected) {
            (Service::TiviOne, false) if self.tivione_too_old() => {
                self.notice = Some((self.strings.tivione_needs_update.to_string(), Instant::now()));
                None
            }
            (Service::TiviOne, connected) => Some(Action::Service(ServiceRequest::TiviOne(!connected))),
            (Service::HomeTiles, _) => {
                self.mode = Mode::Picker(Picker {
                    entities: Vec::new(),
                    selected: HashSet::new(),
                    loading: true,
                    column: 0,
                    category: 0,
                    row: 0,
                    top: 0,
                });
                Some(Action::Service(ServiceRequest::HaCatalog))
            }
            (service, connected) => {
                self.mode = Mode::Setup { service, closing_at: None };
                match (service, connected) {
                    (Service::HomeAssistant, false) => Some(Action::Service(ServiceRequest::HaSetup(true))),
                    (Service::Plex, false) => Some(Action::Service(ServiceRequest::PlexLink(true))),
                    _ => None,
                }
            }
        }
    }

    fn setup_key(&mut self, key: Key) -> Option<Action> {
        let Mode::Setup { service, .. } = self.mode else { return None };
        let connected = match service {
            Service::HomeAssistant => self.status.ha.is_some(),
            Service::Plex => self.status.plex.is_some(),
            Service::Weather => self.status.location.is_some(),
            Service::TiviOne | Service::HomeTiles => false,
        };
        match key {
            Key::Back => {
                self.mode = self.after_overlay();
                match (service, connected) {
                    (Service::HomeAssistant, false) => Some(Action::Service(ServiceRequest::HaSetup(false))),
                    (Service::Plex, false) => Some(Action::Service(ServiceRequest::PlexLink(false))),
                    (Service::Weather, _) => Some(Action::Service(ServiceRequest::LocationSetup(false))),
                    _ => None,
                }
            }
            Key::Ok if connected => {
                self.mode = self.after_overlay();
                Some(Action::Service(match service {
                    Service::HomeAssistant => ServiceRequest::HaDisconnect,
                    Service::Weather => ServiceRequest::LocationClear,
                    _ => ServiceRequest::PlexDisconnect,
                }))
            }
            _ => None,
        }
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
            MenuItem::HidePromo => self.hide_promo(),
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

    // ---- Animation ------------------------------------------------------------------

    fn refocus(&mut self, instant: bool) {
        let moving = matches!(self.mode, Mode::Move { .. });
        let overlay = matches!(self.mode, Mode::Menu(_) | Mode::Setup { .. } | Mode::Offer { .. } | Mode::Picker(_) | Mode::Settings | Mode::Store(_));
        let active = self.active;
        for (r, rail) in self.rails.iter_mut().enumerate() {
            for (i, tile) in rail.tiles.iter_mut().enumerate() {
                tile.focus.target = if r == active && i == rail.focus && !self.header { 1.0 } else { 0.0 };
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
        self.overlay.target = if overlay { 1.0 } else { 0.0 };
        self.lift.target = if moving { 1.0 } else { 0.0 };
        self.vscroll.target = -(self.rail_slot(active) as f32) * RAIL_PITCH;

        let s = self.strings;
        let next = if self.header {
            Some(Hero {
                eyebrow: s.settings.to_uppercase(),
                title: s.settings.to_string(),
                sub: s.settings_sub.to_string(),
                hint: Some(s.press_ok_to_open),
            })
        } else {
            self.focused().map(|tile| self.hero_for(tile, moving))
        };
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

    fn hero_for(&self, tile: &Tile, moving: bool) -> Hero {
        let s = self.strings;
        if let (Kind::Channel { id } | Kind::Reminder { id, .. } | Kind::Catchup { id, .. }, Some(tv)) = (tile.kind, &self.tv) {
            return self.channel_hero(tile, id, tv);
        }
        if tile.kind == Kind::Promo {
            return Hero {
                eyebrow: format!("TiviOne · {}", s.live_tv).to_uppercase(),
                title: s.promo_title.to_string(),
                sub: s.promo_sub.to_string(),
                hint: Some(s.press_ok_for_details),
            };
        }
        let eyebrow = if moving { s.moving.to_uppercase() } else { self.rail_title(self.active).to_uppercase() };
        let (sub, hint) = match (tile.kind, self.active) {
            _ if moving => (s.moving_hint.to_string(), None),
            (Kind::Input { connected: true }, _) => (s.connected_press_ok.to_string(), None),
            (Kind::Input { connected: false }, _) => (s.no_signal.to_string(), None),
            (Kind::App, HIDDEN) => (s.hidden_app.to_string(), None),
            (Kind::App, _) => (s.press_ok_to_open.to_string(), Some(s.hold_ok_for_options)),
            (Kind::Media { .. }, _) => (tile.detail.clone(), Some(s.opens_in_plex)),
            (Kind::Entity { on: None }, _) => (tile.detail.clone(), Some(s.press_ok_to_activate)),
            (Kind::Entity { .. }, _) => (tile.detail.clone(), Some(s.press_ok_to_toggle)),
            (Kind::Setting { service: Service::TiviOne, .. }, _) => (tile.detail.clone(), Some(s.press_ok_to_toggle)),
            (Kind::Setting { service: Service::HomeTiles, .. }, _) => (tile.detail.clone(), Some(s.press_ok_to_manage)),
            (Kind::Setting { connected, .. }, _) => {
                (tile.detail.clone(), Some(if connected { s.press_ok_to_manage } else { s.press_ok_to_connect }))
            }
            (Kind::Channel { .. } | Kind::Reminder { .. } | Kind::Catchup { .. }, _) => (tile.detail.clone(), Some(s.press_ok_to_watch)),
            (Kind::Promo, _) => (tile.detail.clone(), Some(s.press_ok_for_details)),
        };
        Hero { eyebrow, title: tile.title.clone(), sub, hint }
    }

    /// "LIVE · 1 · Das Erste" / programme title / times and what follows.
    fn channel_hero(&self, tile: &Tile, id: i64, tv: &TvFeed) -> Hero {
        let s = self.strings;
        let now_ms = crate::clock::utc_ms();
        let channel = tv.channels.get(&id);
        let name = channel.map(|c| c.name.as_str()).unwrap_or(&tile.title);
        let number = channel.and_then(|c| c.number).map(|n| format!("{n} · ")).unwrap_or_default();
        let (now, next) = tv.now_next(id, now_ms);
        let time = |ms: i64| clock_text(ms);
        if let Kind::Catchup { start, end, .. } = tile.kind {
            return Hero {
                eyebrow: format!("{} · {number}{name}", s.catch_up).to_uppercase(),
                title: tile.title.clone(),
                sub: format!("{} · {} – {}", day_label(s, start), time(start), time(end)),
                hint: Some(s.press_ok_to_watch),
            };
        }
        if let Kind::Reminder { start, .. } = tile.kind {
            return Hero {
                eyebrow: format!("{} · {number}{name}", s.tv_coming).to_uppercase(),
                title: tile.title.clone(),
                sub: time(start),
                hint: Some(s.press_ok_to_watch),
            };
        }
        let title = now.map(|p| p.title.clone()).unwrap_or_else(|| name.to_string());
        let mut sub = now.map(|p| format!("{} – {}", time(p.start), time(p.end))).unwrap_or_default();
        if let Some(next) = next {
            if !sub.is_empty() {
                sub.push_str(" · ");
            }
            sub.push_str(&format!("{} {} {}", s.next, time(next.start), next.title));
        }
        Hero { eyebrow: format!("{} · {number}{name}", s.live).to_uppercase(), title, sub, hint: Some(s.press_ok_to_watch) }
    }

    /// Advances animations; returns true while anything still moves.
    pub fn update(&mut self, dt: f32) -> bool {
        if self.prefs.ui.reduced_motion {
            self.settle();
            if let Some(tile) = self.focused() {
                self.ambient = tile.accent;
            }
        }
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
        self.update_preview();
        if let Some((_, last)) = &self.digits {
            if last.elapsed() >= DIGIT_WAIT {
                if let Some(action) = self.tune_digits() {
                    self.pending.push(action);
                }
            }
            moving = true;
        }
        if let Mode::Setup { closing_at: Some(at), .. } = self.mode {
            if Instant::now() >= at {
                self.mode = self.after_overlay();
                self.refocus(false);
            }
            moving = true;
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

    // ---- Drawing --------------------------------------------------------------------

    pub fn draw(&mut self, r: &mut Renderer, text: &mut Text, now: &chrono::DateTime<chrono::Local>) {
        use chrono::{Datelike, Timelike};
        self.hits.borrow_mut().clear();
        let ui = &self.prefs.ui;
        let clock = if ui.clock24 {
            format!("{:02}:{:02}", now.hour(), now.minute())
        } else {
            let (pm, hour) = now.hour12();
            format!("{hour}:{:02} {}", now.minute(), if pm { "PM" } else { "AM" })
        };
        let s = self.strings;
        let date = if ui.show_date {
            s.date(now.weekday().num_days_from_monday() as usize, now.day(), now.month0() as usize)
        } else {
            String::new()
        };
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
        self.draw_picker(r, text);
        self.draw_digits(r, text);
        self.draw_notice(r, text);
    }

    fn draw_backdrop(&self, r: &mut Renderer) {
        // Ambient light from the focused tile's colour, upper right.
        let strength = [0.0, 0.3, 0.55, 0.8][self.prefs.ui.ambient.min(3) as usize];
        r.radial(1430.0, 280.0, 750.0, 600.0, self.ambient.alpha(strength));
        // Bottom vignette keeps rails legible over the light.
        r.vgradient(Rect::new(0.0, 460.0, LOGICAL_W, 248.0), BG.alpha(0.0), BG.alpha(0.85));
        r.vgradient(Rect::new(0.0, 708.0, LOGICAL_W, 372.0), BG.alpha(0.85), BG);
    }

    fn draw_header(&self, r: &mut Renderer, text: &mut Text, clock: &str, date: &str) {
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

    fn draw_hero(&self, r: &mut Renderer, text: &mut Text) {
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

    fn draw_rails(&self, r: &mut Renderer, text: &mut Text) {
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

    fn channel_now(&self, tile: &Tile) -> Option<(String, f32)> {
        let Kind::Channel { id } = tile.kind else { return None };
        let now_ms = crate::clock::utc_ms();
        let (now, _) = self.tv.as_ref()?.now_next(id, now_ms);
        let now = now?;
        let progress = (now_ms - now.start) as f32 / (now.end - now.start).max(1) as f32;
        Some((now.title.clone(), progress.clamp(0.0, 1.0)))
    }

    fn draw_menu(&self, r: &mut Renderer, text: &mut Text) {
        let open = self.overlay.value.clamp(0.0, 1.0);
        let Mode::Menu(menu) = &self.mode else { return };
        if open <= 0.01 {
            return;
        }
        r.rect(Rect::new(0.0, 0.0, LOGICAL_W, LOGICAL_H), 0.0, BG.alpha(0.6 * open));
        let Some(tile) = self.focused() else { return };
        let rail = &self.rails[self.active];
        let (row_h, pad, width) = (68.0, 16.0, 460.0);
        let height = 64.0 + pad + menu.items.len() as f32 * row_h + pad;
        let title_y = RAIL_TITLE_Y + self.rail_slot(self.active) as f32 * RAIL_PITCH + self.vscroll.value;
        let tile_x = rail.slot_x(rail.focus as f32) - rail.scroll.value;
        let x = tile_x.clamp(SAFE_X, LOGICAL_W - SAFE_X - width);
        let y = title_y - height - 24.0 + 16.0 * (1.0 - open);
        let panel = Rect::new(x, y, width, height);
        draw_panel(r, panel, open);
        let row = |i: usize| Rect::new(x + pad, y + 64.0 + pad + i as f32 * row_h, width - 2.0 * pad, row_h - 8.0);
        for i in 0..menu.items.len() {
            self.hits.borrow_mut().push((row(i), Target::MenuItem(i)));
        }
        r.rect(row(menu.focus), 16.0, TEXT.alpha(open));
        text.draw_top(r, &tile.title, x + 28.0, y + 24.0, MENU_TITLE, TEXT.alpha(0.6 * open));
        for (i, item) in menu.items.iter().enumerate() {
            let label = match item {
                MenuItem::Move => self.strings.menu_move,
                MenuItem::Hide => self.strings.menu_hide,
                MenuItem::Show => self.strings.menu_show,
                MenuItem::HidePromo => self.strings.menu_hide,
                MenuItem::Cancel => self.strings.menu_cancel,
            };
            let color = if i == menu.focus { BG } else { TEXT };
            let rect = row(i);
            text.draw(r, label, rect.x + 22.0, rect.y + 40.0, MENU_ITEM, color.alpha(open));
        }
    }

    fn draw_setup(&self, r: &mut Renderer, text: &mut Text) {
        let open = self.overlay.value.clamp(0.0, 1.0);
        let Mode::Setup { service, .. } = self.mode else { return };
        if open <= 0.01 {
            return;
        }
        let s = self.strings;
        r.rect(Rect::new(0.0, 0.0, LOGICAL_W, LOGICAL_H), 0.0, BG.alpha(0.7 * open));
        let panel = Rect::new(310.0, 230.0 + 20.0 * (1.0 - open), 1300.0, 600.0);
        draw_panel(r, panel, open);
        let (x, mut y) = (panel.x + 64.0, panel.y + 64.0);
        let a = |v: f32| TEXT.alpha(v * open);
        match service {
            Service::HomeAssistant => {
                if let Some(url) = &self.status.ha {
                    text.draw_top(r, s.home_assistant, x, y, SETUP_TITLE, a(1.0));
                    y += 90.0;
                    text.draw_top(r, &format!("{} {url}", s.connected_to), x, y, SETUP_BODY, a(0.8));
                    let button = draw_button(r, text, s.disconnect, x, panel.y + panel.h - 140.0, open);
                    self.hits.borrow_mut().push((button, Target::SetupButton));
                } else {
                    text.draw_top(r, s.ha_setup_title, x, y, SETUP_TITLE, a(1.0));
                    y += 96.0;
                    for (i, step) in s.ha_setup_steps.iter().enumerate() {
                        text.draw_top(r, &format!("{}.", i + 1), x, y, SETUP_BODY, a(0.5));
                        let wrapped = wrap(text, r, step, SETUP_BODY, 640.0);
                        for line in wrapped {
                            text.draw_top(r, &line, x + 44.0, y, SETUP_BODY, a(0.85));
                            y += 42.0;
                        }
                        y += 22.0;
                    }
                    if let Some(error) = &self.status.ha_error {
                        text.draw_top(r, error, x, y + 10.0, SETUP_BODY, Rgba::hex(0xFF8A80).alpha(open));
                    }
                    if let Some((url, qr)) = &self.qr {
                        let side = 380.0;
                        let q = Rect::new(panel.x + panel.w - side - 64.0, panel.y + 64.0, side, side);
                        draw_qr(r, qr, q, open);
                        let w = text.measure(r, url, SETUP_SMALL);
                        text.draw_top(r, url, q.x + (side - w) * 0.5, q.y + side + 22.0, SETUP_SMALL, a(0.6));
                    }
                }
            }
            Service::Weather => {
                let ha = self.status.ha.is_some();
                text.draw_top(r, s.location_title, x, y, SETUP_TITLE, a(1.0));
                y += 96.0;
                for (i, step) in s.location_steps.iter().enumerate() {
                    text.draw_top(r, &format!("{}.", i + 1), x, y, SETUP_BODY, a(0.5));
                    text.draw_top(r, step, x + 44.0, y, SETUP_BODY, a(0.85));
                    y += 64.0;
                }
                if let Some(name) = &self.status.location {
                    text.draw_top(r, &format!("{} {name}", s.current_location), x, y + 10.0, SETUP_BODY, a(0.7));
                    if ha {
                        let button = draw_button(r, text, s.use_ha_location, x, panel.y + panel.h - 140.0, open);
                        self.hits.borrow_mut().push((button, Target::SetupButton));
                    }
                }
                if let Some((url, qr)) = &self.qr {
                    let side = 380.0;
                    let q = Rect::new(panel.x + panel.w - side - 64.0, panel.y + 64.0, side, side);
                    draw_qr(r, qr, q, open);
                    let w = text.measure(r, url, SETUP_SMALL);
                    text.draw_top(r, url, q.x + (side - w) * 0.5, q.y + side + 22.0, SETUP_SMALL, a(0.6));
                }
            }
            Service::Plex | Service::TiviOne | Service::HomeTiles => {
                if let Some(account) = &self.status.plex {
                    text.draw_top(r, s.plex, x, y, SETUP_TITLE, a(1.0));
                    y += 90.0;
                    text.draw_top(r, &format!("{} {account}", s.connected_to), x, y, SETUP_BODY, a(0.8));
                    let button = draw_button(r, text, s.disconnect, x, panel.y + panel.h - 140.0, open);
                    self.hits.borrow_mut().push((button, Target::SetupButton));
                } else {
                    text.draw_top(r, s.plex_setup_title, x, y, SETUP_TITLE, a(1.0));
                    y += 96.0;
                    text.draw_top(r, s.plex_setup_step, x, y, SETUP_BODY, a(0.85));
                    y += 90.0;
                    match (&self.status.plex_code, &self.status.plex_error) {
                        (_, Some(error)) => {
                            text.draw_top(r, error, x, y, SETUP_BODY, Rgba::hex(0xFF8A80).alpha(open));
                        }
                        (Some(code), None) => {
                            text.draw_top(r, code, x - 6.0, y, SETUP_CODE, a(1.0));
                            text.draw_top(r, s.waiting_for_plex, x, y + 180.0, SETUP_BODY, a(0.55));
                        }
                        (None, None) => {
                            text.draw_top(r, s.waiting_for_plex, x, y, SETUP_BODY, a(0.55));
                        }
                    }
                }
            }
        }
        let hint_w = text.measure(r, s.back_to_close, SETUP_SMALL);
        text.draw_top(r, s.back_to_close, panel.x + panel.w - 64.0 - hint_w, panel.y + panel.h - 64.0, SETUP_SMALL, a(0.45));
    }

    fn draw_offer(&self, r: &mut Renderer, text: &mut Text) {
        let open = self.overlay.value.clamp(0.0, 1.0);
        let Mode::Offer { yes } = self.mode else { return };
        if open <= 0.01 {
            return;
        }
        let s = self.strings;
        r.rect(Rect::new(0.0, 0.0, LOGICAL_W, LOGICAL_H), 0.0, BG.alpha(0.6 * open));
        let lines = wrap(text, r, s.offer_body, SETUP_BODY, 800.0 - 112.0);
        let height = 140.0 + lines.len() as f32 * 42.0 + 48.0 + 64.0 + 52.0;
        let panel = Rect::new(560.0, (LOGICAL_H - height) * 0.5 + 20.0 * (1.0 - open), 800.0, height);
        draw_panel(r, panel, open);
        let x = panel.x + 56.0;
        text.draw_top(r, s.offer_title, x, panel.y + 52.0, SETUP_TITLE, TEXT.alpha(open));
        let mut y = panel.y + 140.0;
        for line in &lines {
            text.draw_top(r, line, x, y, SETUP_BODY, TEXT.alpha(0.8 * open));
            y += 42.0;
        }
        let by = panel.y + panel.h - 52.0 - 64.0;
        let mut bx = x;
        for (label, focused) in [(s.turn_on, yes), (s.not_now, !yes)] {
            let w = text.measure(r, label, MENU_ITEM) + 72.0;
            let rect = Rect::new(bx, by, w, 64.0);
            self.hits.borrow_mut().push((rect, Target::Offer(label == s.turn_on)));
            if focused {
                r.rect(rect, 32.0, TEXT.alpha(open));
            } else {
                r.stroke(rect, 32.0, 1.5, TEXT.alpha(0.35 * open));
            }
            text.draw(r, label, bx + 36.0, by + 42.0, MENU_ITEM, if focused { BG } else { TEXT }.alpha(open));
            bx += w + 20.0;
        }
    }

    /// TiviOne's store page: banner and facts, what it does, screenshots.
    fn draw_store(&self, r: &mut Renderer, text: &mut Text) {
        let open = self.overlay.value.clamp(0.0, 1.0);
        let Mode::Store(store) = &self.mode else { return };
        if open <= 0.01 {
            return;
        }
        let s = self.strings;
        let a = |v: f32| TEXT.alpha(v * open);
        let shots = self.release.as_ref().map_or(0, |r| r.pictures.len().saturating_sub(1));
        r.rect(Rect::new(0.0, 0.0, LOGICAL_W, LOGICAL_H), 0.0, BG.alpha(open));
        if store.full {
            match self.picture(store.shot + 1) {
                Some((texture, uv)) => r.image(texture, Rect::new(0.0, 0.0, LOGICAL_W, LOGICAL_H), uv, 0.0, Rgba::hex(0xFFFFFF).alpha(open)),
                None => {
                    text.draw_top(r, s.store_screenshots, SAFE_X, SAFE_TOP, RAIL_TITLE, a(0.6));
                }
            }
            let counter = format!("{} / {shots}", store.shot + 1);
            let w = text.measure(r, &counter, HINT) + 40.0;
            let pill = Rect::new(LOGICAL_W - SAFE_X - w, LOGICAL_H - SAFE_TOP - 52.0, w, 52.0);
            r.rect(pill, 26.0, BG.alpha(0.75 * open));
            text.draw(r, &counter, pill.x + 20.0, pill.y + 35.0, HINT, a(0.9));
            return;
        }
        r.radial(1500.0, 160.0, 900.0, 620.0, TIVIONE_TEAL.alpha(0.2 * open));
        let dy = 24.0 * (1.0 - open);

        let banner = Rect::new(SAFE_X, 96.0 + dy, 746.0, 420.0);
        r.shadow(banner.offset(0.0, 24.0), TILE_RADIUS, 60.0, Rgba::hex(0).alpha(0.5 * open));
        match self.picture(0) {
            Some((texture, uv)) => r.image(texture, banner, uv, TILE_RADIUS, Rgba::hex(0xFFFFFF).alpha(open)),
            None => {
                r.rect(banner, TILE_RADIUS, Rgba::hex(0x062A2E).alpha(open));
                let w = text.measure(r, "TiviOne", SETUP_TITLE);
                text.draw_top(r, "TiviOne", banner.x + (banner.w - w) * 0.5, banner.y + banner.h * 0.5 - 34.0, SETUP_TITLE, a(1.0));
            }
        }

        // Name, pitch, facts and the buttons, right of the banner.
        let x = banner.x + banner.w + 64.0;
        let width = LOGICAL_W - SAFE_X - x;
        let mut y = banner.y + 6.0;
        text.draw_top(r, &format!("TiviOne · {}", s.live_tv).to_uppercase(), x, y, EYEBROW, TIVIONE_TEAL.alpha(open));
        y += 44.0;
        let title = crate::theme::hero(HERO_SIZES[2]);
        text.draw_top(r, "TiviOne", x - 4.0, y, title, a(1.0));
        y += title.size * 1.08 + 16.0;
        for line in wrap(text, r, s.store_tagline, HERO_SUB, width).iter().take(3) {
            text.draw_top(r, line, x, y, HERO_SUB, a(0.78));
            y += 40.0;
        }
        y += 8.0;
        let mut facts = Vec::new();
        if let Some(release) = &self.release {
            facts.push(format!("{} {}", s.version, release.version));
            facts.push(format!("{:.1} MB", release.size as f32 / 1_048_576.0));
        }
        if let Some(installed) = &self.tivione {
            facts.push(format!("{} {}", s.store_installed, installed.version));
        }
        facts.push("tivione.zf-labs.de".into());
        text.draw_top(r, &facts.join(" · "), x, y, SETUP_SMALL, a(0.55));
        y += 52.0;

        let mut bx = x;
        let progress = match self.install {
            InstallState::Downloading(p) => Some((p, format!("{} {:.0} %", s.store_downloading, p * 100.0))),
            InstallState::Installing => Some((1.0, s.store_installing.to_string())),
            _ => None,
        };
        if let Some((p, label)) = progress {
            let track = Rect::new(bx, y, 440.0, 64.0);
            r.rect(track, 32.0, TEXT.alpha(0.1 * open));
            r.rect(Rect::new(track.x, track.y, (track.w * p).max(64.0), track.h), 32.0, TIVIONE_TEAL.alpha(0.85 * open));
            text.draw(r, &label, track.x + 32.0, track.y + 42.0, MENU_ITEM, a(1.0));
            bx += track.w + 20.0;
        }
        let buttons = self.store_buttons();
        let focus = store.button.min(buttons.len().saturating_sub(1));
        for (i, button) in buttons.iter().enumerate() {
            let label = match button {
                StoreButton::Install => s.store_install,
                StoreButton::Update => s.store_update,
                StoreButton::Retry => s.store_retry,
                StoreButton::Open => s.store_open,
                StoreButton::Hide => s.store_hide,
                StoreButton::Close => s.store_close,
            };
            let w = text.measure(r, label, MENU_ITEM) + 72.0;
            let rect = Rect::new(bx, y, w, 64.0);
            self.hits.borrow_mut().push((rect, Target::StoreButton(i)));
            let focused = store.row == 0 && i == focus;
            if focused {
                r.rect(rect, 32.0, TEXT.alpha(open));
            } else {
                r.stroke(rect, 32.0, 1.5, TEXT.alpha(0.35 * open));
            }
            text.draw(r, label, bx + 36.0, y + 42.0, MENU_ITEM, if focused { BG } else { TEXT }.alpha(open));
            bx += w + 20.0;
        }
        let status = match (&self.install, &self.tivione, &self.release) {
            (InstallState::Failed(failure), ..) => Some((
                match failure {
                    InstallFailure::Network => s.fail_network,
                    InstallFailure::Damaged => s.fail_damaged,
                    InstallFailure::NeedsRoot => s.fail_root,
                    InstallFailure::Refused => s.fail_refused,
                },
                Rgba::hex(0xFF8A80),
            )),
            (_, None, None) => Some((s.store_unavailable, TEXT.alpha(0.6))),
            (_, Some(i), Some(release)) if !i.direct && version_newer(&release.version, &i.version) => Some((s.store_lg_updates, TEXT.alpha(0.6))),
            _ => None,
        };
        if let Some((message, color)) = status {
            let line = clip(text, r, message, SETUP_SMALL, width);
            text.draw_top(r, &line, x, y + 88.0, SETUP_SMALL, color.fade(open));
        }

        // What it does: two columns under the banner.
        let top = banner.y + banner.h + 52.0;
        let column = (LOGICAL_W - 2.0 * SAFE_X - 64.0) * 0.5;
        for (i, feature) in s.store_features.iter().enumerate() {
            let (fx, fy) = (SAFE_X + (i % 2) as f32 * (column + 64.0), top + (i / 2) as f32 * 108.0);
            r.rect(Rect::new(fx, fy + 12.0, 12.0, 12.0), 6.0, TIVIONE_TEAL.alpha(open));
            for (l, line) in wrap(text, r, feature, HERO_SUB, column - 36.0).iter().take(2).enumerate() {
                text.draw_top(r, line, fx + 32.0, fy + l as f32 * 40.0, HERO_SUB, a(0.85));
            }
        }

        // Screenshots: OK shows one full screen.
        let title_y = top + 2.0 * 108.0 + 6.0;
        text.draw_top(r, s.store_screenshots, SAFE_X, title_y, RAIL_TITLE, a(if store.row == 1 { 0.7 } else { 0.5 }));
        let (cw, ch, gap) = (320.0, 180.0, 28.0);
        for i in 0..shots {
            let base = Rect::new(SAFE_X + i as f32 * (cw + gap), title_y + 44.0, cw, ch);
            let focused = store.row == 1 && i == store.shot;
            let card = if focused { base.scale_about(1.06, 0.5, 0.5) } else { base };
            self.hits.borrow_mut().push((card, Target::StoreShot(i)));
            match self.picture(i + 1) {
                Some((texture, uv)) => r.image(texture, card, uv, 14.0, Rgba::hex(0xFFFFFF).alpha((if focused { 1.0 } else { 0.78 }) * open)),
                None => r.rect(card, 14.0, Rgba::hex(0x14161C).alpha(open)),
            }
            if focused {
                r.stroke(card.inflate(RING_OFFSET + RING_WIDTH * 0.5), 14.0 + RING_OFFSET, RING_WIDTH, a(1.0));
            }
        }
    }

    fn draw_settings(&self, r: &mut Renderer, text: &mut Text) {
        let open = self.overlay.value.clamp(0.0, 1.0);
        if !matches!(self.mode, Mode::Settings) || open <= 0.01 {
            return;
        }
        let s = self.strings;
        let a = |v: f32| TEXT.alpha(v * open);
        r.rect(Rect::new(0.0, 0.0, LOGICAL_W, LOGICAL_H), 0.0, BG.alpha(0.82 * open));
        let panel = Rect::new(210.0, 120.0 + 16.0 * (1.0 - open), 1500.0, 840.0);
        draw_panel(r, panel, open);
        let (x, y) = (panel.x + 56.0, panel.y + 48.0);
        text.draw_top(r, s.settings, x, y, SETUP_TITLE, a(1.0));
        let hint = if self.settings_column == 0 { s.settings_hint } else { s.settings_list_hint };
        let hint_w = text.measure(r, hint, SETUP_SMALL);
        text.draw_top(r, hint, panel.x + panel.w - 56.0 - hint_w, y + 22.0, SETUP_SMALL, a(0.5));

        let sections = self.settings_sections();
        let top = panel.y + 150.0;
        let (section_w, row_h) = (360.0, 72.0);
        for (i, (name, _)) in sections.iter().enumerate() {
            let row = Rect::new(x - 16.0, top + i as f32 * row_h, section_w, row_h - 10.0);
            self.hits.borrow_mut().push((row, Target::SettingsSection(i)));
            let current = i == self.settings_section;
            let focused = current && self.settings_column == 0;
            if focused {
                r.rect(row, 14.0, TEXT.alpha(open));
            } else if current {
                r.rect(row, 14.0, TEXT.alpha(0.08 * open));
            }
            text.draw(r, name, row.x + 20.0, row.y + 42.0, MENU_ITEM, if focused { BG } else { TEXT }.alpha(open));
        }

        let list_x = x + section_w + 40.0;
        let list_w = panel.x + panel.w - 56.0 - list_x;
        let Some((_, items)) = sections.get(self.settings_section) else { return };
        let item_h = 84.0;
        for (i, item) in items.iter().enumerate() {
            let row = Rect::new(list_x, top + i as f32 * item_h, list_w, item_h - 12.0);
            if item.enabled {
                self.hits.borrow_mut().push((row, Target::SettingsRow(i)));
            }
            let focused = self.settings_column == 1 && i == self.settings_row;
            if focused {
                r.rect(row, 16.0, TEXT.alpha(open));
            }
            let color = if focused { BG } else { TEXT };
            let dim = if item.enabled { 1.0 } else { 0.55 };
            text.draw(r, &item.label, row.x + 24.0, row.y + 46.0, MENU_ITEM, color.alpha(open * dim));
            let mut right = row.x + row.w - 24.0;
            if let Some(on) = item.on {
                // Switch: a pill track with a knob.
                let track = Rect::new(right - 60.0, row.y + (row.h - 32.0) * 0.5, 60.0, 32.0);
                let fill = if on { Rgba::hex(0x4ADE80) } else { color.alpha(0.25) };
                r.rect(track, 16.0, fill.fade(open));
                let knob_x = if on { track.x + track.w - 28.0 } else { track.x + 4.0 };
                r.rect(Rect::new(knob_x, track.y + 4.0, 24.0, 24.0), 12.0, Rgba::hex(0xFFFFFF).alpha(open));
                right = track.x - 20.0;
            }
            if !item.value.is_empty() && item.on.is_none() || item.on.is_some() && !matches!(item.id, SettingId::Row(_) | SettingId::WeatherShow | SettingId::ShowDate | SettingId::HomeTakeover) {
                let value = clip(text, r, &item.value, INPUT_SUB, list_w * 0.5);
                let w = text.measure(r, &value, INPUT_SUB);
                text.draw(r, &value, right - w, row.y + 44.0, INPUT_SUB, color.alpha(0.6 * open));
            }
        }
        if self.settings_section == 3 {
            let y = top + items.len() as f32 * item_h + 16.0;
            for (i, line) in wrap(text, r, s.home_button_note, INPUT_SUB, list_w - 48.0).iter().enumerate() {
                text.draw_top(r, line, list_x + 24.0, y + i as f32 * 30.0, INPUT_SUB, a(0.5));
            }
        }
    }

    fn draw_picker(&self, r: &mut Renderer, text: &mut Text) {
        let open = self.overlay.value.clamp(0.0, 1.0);
        let Mode::Picker(p) = &self.mode else { return };
        if open <= 0.01 {
            return;
        }
        let s = self.strings;
        let a = |v: f32| TEXT.alpha(v * open);
        r.rect(Rect::new(0.0, 0.0, LOGICAL_W, LOGICAL_H), 0.0, BG.alpha(0.82 * open));
        let panel = Rect::new(150.0, 90.0 + 16.0 * (1.0 - open), 1620.0, 900.0);
        draw_panel(r, panel, open);
        let (x, y) = (panel.x + 56.0, panel.y + 48.0);
        text.draw_top(r, s.home_tiles, x, y, SETUP_TITLE, a(1.0));
        let hint_w = text.measure(r, s.picker_hint, SETUP_SMALL);
        text.draw_top(r, s.picker_hint, panel.x + panel.w - 56.0 - hint_w, y + 22.0, SETUP_SMALL, a(0.5));
        let top = panel.y + 150.0;
        if p.loading {
            text.draw_top(r, s.loading, x, top, SETUP_BODY, a(0.6));
            return;
        }

        // Categories.
        let (cat_w, row_h) = (400.0, 66.0);
        for (i, category) in HaCategory::ALL.iter().enumerate() {
            let row = Rect::new(x - 16.0, top + i as f32 * row_h, cat_w, row_h - 8.0);
            self.hits.borrow_mut().push((row, Target::PickerCategory(i)));
            let current = i == p.category;
            let focused = current && p.column == 0;
            if focused {
                r.rect(row, 14.0, TEXT.alpha(open));
            } else if current {
                r.rect(row, 14.0, TEXT.alpha(0.08 * open));
            }
            let color = if focused { BG } else { TEXT };
            let (chosen, total) = p.count(*category);
            text.draw(r, s.categories[i], row.x + 20.0, row.y + 40.0, MENU_ITEM, color.alpha(open * if total == 0 { 0.4 } else { 1.0 }));
            let count = format!("{chosen}/{total}");
            let w = text.measure(r, &count, INPUT_SUB);
            text.draw(r, &count, row.x + row.w - 20.0 - w, row.y + 38.0, INPUT_SUB, color.alpha(0.6 * open));
        }

        // Entities of the current category.
        let list_x = x + cat_w + 40.0;
        let list_w = panel.x + panel.w - 56.0 - list_x;
        let rows = p.rows();
        if rows.is_empty() {
            text.draw_top(r, s.none_here, list_x, top + 12.0, SETUP_BODY, a(0.5));
            return;
        }
        for (slot, &index) in rows.iter().enumerate().skip(p.top).take(PICKER_ROWS) {
            let entity = &p.entities[index];
            let row = Rect::new(list_x, top + (slot - p.top) as f32 * row_h, list_w, row_h - 8.0);
            self.hits.borrow_mut().push((row, Target::PickerRow(slot)));
            let focused = p.column == 1 && slot == p.row;
            if focused {
                r.rect(row, 14.0, TEXT.alpha(open));
            }
            let color = if focused { BG } else { TEXT };
            // Checkbox: filled with a centre mark when chosen.
            let tick = Rect::new(row.x + 20.0, row.y + (row.h - 28.0) * 0.5, 28.0, 28.0);
            if p.selected.contains(&entity.id) {
                r.rect(tick, 7.0, Rgba::hex(0xF2C14E).alpha(open));
                r.rect(tick.inflate(-8.0), 3.0, Rgba::hex(0x07080B).alpha(open));
            } else {
                r.stroke(tick, 7.0, 2.0, color.alpha(0.6 * open));
            }
            let name_x = tick.x + tick.w + 22.0;
            let state = entity_state(s, entity);
            let state_w = text.measure(r, &state, INPUT_SUB);
            let area_w = if entity.area.is_empty() { 0.0 } else { text.measure(r, &entity.area, INPUT_SUB) + 32.0 };
            let name = clip(text, r, &entity.name, MENU_ITEM, row.x + row.w - 24.0 - state_w - 32.0 - area_w - name_x);
            text.draw(r, &name, name_x, row.y + 40.0, MENU_ITEM, color.alpha(open));
            let mut right = row.x + row.w - 24.0;
            text.draw(r, &state, right - state_w, row.y + 38.0, INPUT_SUB, color.alpha(0.55 * open));
            right -= state_w + 32.0;
            if !entity.area.is_empty() {
                text.draw(r, &entity.area, right - (area_w - 32.0), row.y + 38.0, INPUT_SUB, color.alpha(0.75 * open));
            }
        }
        // Scroll hint: how far down the list we are.
        if rows.len() > PICKER_ROWS {
            let track = Rect::new(panel.x + panel.w - 30.0, top, 4.0, PICKER_ROWS as f32 * row_h - 8.0);
            r.rect(track, 2.0, TEXT.alpha(0.1 * open));
            let h = track.h * PICKER_ROWS as f32 / rows.len() as f32;
            let y = track.y + (track.h - h) * p.top as f32 / (rows.len() - PICKER_ROWS) as f32;
            r.rect(Rect::new(track.x, y, 4.0, h), 2.0, TEXT.alpha(0.5 * open));
        }
    }

    /// The channel number being typed, bottom right ("1 2 _").
    fn draw_digits(&self, r: &mut Renderer, text: &mut Text) {
        let Some((entry, _)) = &self.digits else { return };
        let label: String = entry.chars().map(|c| format!("{c} ")).collect::<String>() + "_";
        let w = text.measure(r, &label, DIGITS) + 64.0;
        let rect = Rect::new(LOGICAL_W - SAFE_X - w, LOGICAL_H - 54.0 - 96.0, w, 96.0);
        r.rect(rect, 48.0, Rgba::hex(0x14161C).alpha(0.94));
        r.stroke(rect, 48.0, 1.0, TEXT.alpha(0.16));
        text.draw(r, &label, rect.x + 32.0, rect.y + 66.0, DIGITS, TEXT);
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
    /// Guide position for channel tiles: (programme title, progress).
    now: Option<(String, f32)>,
    rect: Rect,
    f: f32,
    alpha: f32,
    lifted: f32,
    icon: Option<[f32; 5]>,
    art: Option<[f32; 4]>,
    fill: Option<Rgba>,
}

/// Top-left texel of a store picture's cell (two per row).
fn picture_cell(index: usize) -> (i32, i32) {
    ((index % 2) as i32 * PICTURE_CELL.0, (index / 2) as i32 * PICTURE_CELL.1)
}

fn draw_panel(r: &mut Renderer, panel: Rect, open: f32) {
    r.shadow(panel.offset(0.0, 24.0), 22.0, 60.0, Rgba::hex(0).alpha(0.5 * open));
    r.rect(panel, 22.0, Rgba::hex(0x14161C).alpha(0.96 * open));
    r.stroke(panel, 22.0, 1.0, TEXT.alpha(0.14 * open));
}

fn draw_button(r: &mut Renderer, text: &mut Text, label: &str, x: f32, y: f32, open: f32) -> Rect {
    let w = text.measure(r, label, MENU_ITEM) + 72.0;
    let rect = Rect::new(x, y, w, 64.0);
    r.rect(rect, 32.0, TEXT.alpha(open));
    text.draw(r, label, x + 36.0, y + 42.0, MENU_ITEM, BG.alpha(open));
    rect
}

/// QR modules as merged horizontal runs on a white quiet-zone card.
fn draw_qr(r: &mut Renderer, qr: &qrcodegen::QrCode, area: Rect, open: f32) {
    r.rect(area, 18.0, Rgba::hex(0xFFFFFF).alpha(open));
    let n = qr.size();
    let quiet = 3;
    let module = area.w / (n + 2 * quiet) as f32;
    let ink = Rgba::hex(0x07080B).alpha(open);
    for y in 0..n {
        let mut x = 0;
        while x < n {
            if qr.get_module(x, y) {
                let start = x;
                while x < n && qr.get_module(x, y) {
                    x += 1;
                }
                let rect = Rect::new(
                    area.x + (start + quiet) as f32 * module,
                    area.y + (y + quiet) as f32 * module,
                    (x - start) as f32 * module + 0.5,
                    module + 0.5,
                );
                r.rect(rect, 0.0, ink);
            } else {
                x += 1;
            }
        }
    }
}

/// Greedy word wrap to `width`.
fn wrap(text: &mut Text, r: &Renderer, line: &str, style: Style, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in line.split(' ') {
        let candidate = if current.is_empty() { word.to_string() } else { format!("{current} {word}") };
        if !current.is_empty() && text.measure(r, &candidate, style) > width {
            lines.push(std::mem::replace(&mut current, word.to_string()));
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn draw_hero_block(r: &mut Renderer, text: &mut Text, hero: &Hero, alpha: f32, dy: f32, accent: Rgba, width: f32) {
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

fn draw_tile_shapes(r: &mut Renderer, tile: &Tile, v: &TileView) {
    let (rect, f, alpha) = (v.rect, v.f, v.alpha);
    if f > 0.0 {
        r.shadow(rect, TILE_RADIUS * 2.0, 80.0, tile.accent.alpha(0.45 * f * alpha));
        r.shadow(rect.offset(0.0, 30.0 + 16.0 * v.lifted), TILE_RADIUS, 60.0 + 30.0 * v.lifted, Rgba::hex(0).alpha(0.55 * f * alpha));
    }
    let fill = match (v.art, v.fill) {
        (None, Some(edge)) => edge,
        _ => tile.bg,
    };
    r.rect(rect, TILE_RADIUS, fill.fade(alpha));
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

fn draw_tile_art(r: &mut Renderer, atlas: Texture, v: &TileView) {
    let Some(uv) = v.art else { return };
    // Unfocused artwork sits back slightly so the focused tile reads first.
    let light = 0.78 + 0.22 * v.f;
    r.image(atlas, v.rect, uv, TILE_RADIUS, Rgba { r: light, g: light, b: light, a: v.alpha });
}

fn draw_tile_icon(r: &mut Renderer, atlas: Texture, v: &TileView) {
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

fn draw_tile_text(r: &mut Renderer, text: &mut Text, tile: &Tile, v: &TileView, s: &Strings) {
    let (rect, f, alpha) = (v.rect, v.f, v.alpha);
    if f < 1.0 {
        r.stroke(rect.inflate(-0.5), TILE_RADIUS, 1.0, TEXT.alpha(0.10 * (1.0 - f) * alpha));
    }
    if f > 0.0 {
        let ring = TEXT.lerp(tile.accent, v.lifted);
        r.stroke(rect.inflate(RING_OFFSET + RING_WIDTH * 0.5), TILE_RADIUS + RING_OFFSET, RING_WIDTH, ring.alpha(f * alpha));
    }
    let label_below = |text: &mut Text, r: &mut Renderer| {
        if f > 0.01 {
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
fn draw_gear(r: &mut Renderer, area: Rect, color: Rgba) {
    let (cx, cy) = (area.x + area.w * 0.5, area.y + area.h * 0.5);
    for i in 0..8 {
        let angle = i as f32 * std::f32::consts::FRAC_PI_4;
        let (tx, ty) = (cx + angle.cos() * 15.0, cy + angle.sin() * 15.0);
        r.rect(Rect::new(tx - 4.5, ty - 4.5, 9.0, 9.0), 3.0, color);
    }
    r.stroke(Rect::new(cx - 11.0, cy - 11.0, 22.0, 22.0), 11.0, 6.0, color);
}

/// "On · 40 %", "Open", "Playing", "Scene" … for an entity.
fn entity_state(s: &Strings, e: &Entity) -> String {
    match e.category {
        HaCategory::Scenes => s.scene.to_string(),
        HaCategory::Scripts => s.script.to_string(),
        HaCategory::Covers => match e.state.as_str() {
            "open" | "opening" => s.open.to_string(),
            "closed" | "closing" => s.closed.to_string(),
            other => other.to_string(),
        },
        HaCategory::Media => match e.state.as_str() {
            "playing" => s.playing.to_string(),
            "paused" => s.paused.to_string(),
            "off" | "standby" => s.off.to_string(),
            other => other.to_string(),
        },
        _ => match (e.on, e.brightness) {
            (Some(true), Some(b)) => format!("{} · {b} %", s.on),
            (Some(true), None) => s.on.to_string(),
            _ => s.off.to_string(),
        },
    }
}

/// "Today", "Yesterday" or the date, for a UTC timestamp in ms.
fn day_label(s: &Strings, ms: i64) -> String {
    use chrono::{Datelike, TimeZone};
    let Some(then) = chrono::Local.timestamp_millis_opt(ms).single() else { return String::new() };
    let today = crate::clock::local().date_naive();
    match (today - then.date_naive()).num_days() {
        0 => s.today.to_string(),
        1 => s.yesterday.to_string(),
        _ => s.date(then.weekday().num_days_from_monday() as usize, then.day(), then.month0() as usize),
    }
}

/// Local wall-clock "HH:MM" for a UTC timestamp in ms.
fn clock_text(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ms).single().map(|t| t.format("%H:%M").to_string()).unwrap_or_default()
}

/// Shortens `label` with an ellipsis to fit `width`.
fn clip(text: &mut Text, r: &Renderer, label: &str, style: Style, width: f32) -> String {
    if text.measure(r, label, style) <= width {
        return label.to_string();
    }
    let mut chars: Vec<char> = label.chars().collect();
    while chars.len() > 1 {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>().trim_end().to_string() + "…";
        if text.measure(r, &candidate, style) <= width {
            return candidate;
        }
    }
    label.to_string()
}

fn fit(text: &mut Text, r: &Renderer, label: &str, style: Style, width: f32) -> Style {
    let mut style = style;
    while style.size > 20.0 && text.measure(r, label, style) > width {
        style.size -= 4.0;
    }
    style
}
