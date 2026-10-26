//! Home screen state, input and drawing (direction A, "Cinematic Ambient").
mod chrome;
mod cinematic;
mod glass;
mod homebrew;
mod nav;
mod overlays;

use chrome::*;

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
    /// Switch the look (fonts load first; the event loop then calls `set_look`).
    SetTheme(Look),
    /// Load a Glass wallpaper: a bundled one by name, or `custom`.
    Wallpaper(String),
    /// Load the Homebrew repository; install or uninstall one of its apps.
    Homebrew,
    HomebrewInstall(crate::model::Package),
    HomebrewRemove(String),
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
    Wallpaper,
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
    /// Glass grid position (column, row); animates when apps are moved.
    gcol: Spring,
    grow: Spring,
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
            gcol: Spring::new((index % glass::COLS) as f32, 16.0),
            grow: Spring::new((index / glass::COLS) as f32, 16.0),
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
    gap: f32,
}

impl Rail {
    fn new(tile_w: f32, tile_h: f32) -> Self {
        Self { tiles: Vec::new(), focus: 0, scroll: Spring::new(0.0, 16.0), tile_w, tile_h, gap: TILE_GAP }
    }

    fn slot_x(&self, slot: f32) -> f32 {
        SAFE_X + slot * (self.tile_w + self.gap)
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
    /// The Homebrew apps page.
    Homebrew(homebrew::Brew),
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
    /// The look: Cinematic or Glass.
    Theme,
    /// Glass wallpaper, and sending a photo from the phone.
    Wallpaper,
    WallpaperUpload,
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
    BrewTab(usize),
    BrewCard(usize),
    BrewButton(usize),
}

struct Hero {
    eyebrow: String,
    title: String,
    sub: String,
    hint: Option<&'static str>,
}

pub struct Home {
    look: Look,
    /// Glass: Top Shelf opened (focus in the shelf), its scroll, the Settings tile's focus and
    /// the backdrop (focused app) with a crossfade from the previous one.
    shelf: Spring,
    shelf_scroll: Spring,
    settings_focus: Spring,
    backdrop: Option<String>,
    previous_backdrop: Option<String>,
    backdrop_t: Spring,
    /// Glass parallax: the focused tile's tilt (-1..1), from the pointer or a short kick when
    /// focus moves; and the wiggle clock while moving apps.
    tilt_x: Spring,
    tilt_y: Spring,
    wiggle: f32,
    /// Glass wallpaper and its blurred copy (for the dock), once loaded.
    wallpaper: Option<(Texture, Texture)>,
    /// The Homebrew page: the repository (None while loading), icons, running installs, and
    /// every installed app's version.
    brew: Option<Result<Vec<crate::model::HomebrewApp>, String>>,
    brew_icons: HashMap<String, [f32; 4]>,
    brew_atlas: Option<Texture>,
    brew_next: i32,
    brew_progress: HashMap<String, InstallState>,
    brew_scroll: Spring,
    versions: HashMap<String, String>,
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
    pub fn new(renderer: &Renderer, prefs: Prefs, look: Look) -> Self {
        let mut home = Self {
            look,
            shelf: Spring::new(0.0, 14.0),
            shelf_scroll: Spring::new(0.0, 16.0),
            settings_focus: Spring::new(0.0, 18.0),
            backdrop: None,
            previous_backdrop: None,
            backdrop_t: Spring::new(1.0, 8.0),
            wallpaper: None,
            brew: None,
            brew_icons: HashMap::new(),
            brew_atlas: None,
            brew_next: 0,
            brew_progress: HashMap::new(),
            brew_scroll: Spring::new(0.0, 16.0),
            versions: HashMap::new(),
            tilt_x: Spring::new(0.0, 7.0),
            tilt_y: Spring::new(0.0, 7.0),
            wiggle: 0.0,
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
        };
        home.apply_look();
        glass::upload_sheen(renderer, home.art_atlas);
        if home.look == Look::Glass {
            home.pending.push(Action::Wallpaper(home.prefs.ui.wallpaper.clone()));
        }
        home
    }

    /// Switches the look once its fonts are loaded (Settings stays open).
    pub fn set_look(&mut self, look: Look) {
        self.look = look;
        self.apply_look();
        self.glass_reslot();
        self.hero = None;
        self.previous_hero = None;
        self.backdrop = None;
        self.previous_backdrop = None;
        self.settle();
        self.refocus(true);
        if look == Look::Glass && self.wallpaper.is_none() {
            self.pending.push(Action::Wallpaper(self.prefs.ui.wallpaper.clone()));
        }
    }

    /// Bundled wallpapers, then the phone photo when there is one.
    fn wallpapers(&self) -> Vec<&'static str> {
        let mut names = vec!["valley", "crete", "dunes"];
        if self.status.wallpaper_custom {
            names.push("custom");
        }
        names
    }

    fn wallpaper_name(&self, id: &str) -> &'static str {
        let s = self.strings;
        match id {
            "crete" => s.wallpaper_names[1],
            "dunes" => s.wallpaper_names[2],
            "custom" => s.wallpaper_own,
            _ => s.wallpaper_names[0],
        }
    }

    /// The panel behind menus and dialogs, in the look's style.
    fn panel(&self, r: &mut Renderer, rect: Rect, open: f32) {
        match self.look {
            Look::Cinematic => draw_panel(r, rect, open),
            Look::Glass => {
                r.shadow(rect.offset(0.0, 30.0), 28.0, 80.0, Rgba::hex(0).alpha(0.6 * open));
                r.rect(rect, 28.0, Rgba::hex(0x1E1E24).alpha(0.97 * open));
            }
        }
    }

    /// Tile sizes and gaps per look (Cinematic keeps its original values).
    fn apply_look(&mut self) {
        let sizes: [(f32, f32, f32); 9] = match self.look {
            Look::Cinematic => [
                (300.0, 170.0, TILE_GAP),
                (300.0, 170.0, TILE_GAP),
                (300.0, 170.0, TILE_GAP),
                (300.0, 170.0, TILE_GAP),
                (300.0, 150.0, TILE_GAP),
                (300.0, 150.0, TILE_GAP),
                (300.0, 150.0, TILE_GAP),
                (300.0, 120.0, TILE_GAP),
                (300.0, 170.0, TILE_GAP),
            ],
            Look::Glass => {
                let card = (glass::CARD.0, glass::CARD.1, glass::CARD_GAP);
                [
                    (glass::TILE.0, glass::TILE.1, glass::GAP),
                    card,
                    (glass::TILE.0, glass::TILE.1, glass::ROW_GAP),
                    card,
                    card,
                    card,
                    (glass::TILE.0, 130.0, glass::ROW_GAP),
                    (glass::TILE.0, 100.0, glass::ROW_GAP),
                    (glass::TILE.0, glass::TILE.1, glass::ROW_GAP),
                ]
            }
        };
        for (rail, (w, h, gap)) in self.rails.iter_mut().zip(sizes) {
            rail.tile_w = w;
            rail.tile_h = h;
            rail.gap = gap;
        }
    }

    // ---- Magic Remote pointer ---------------------------------------------------------

    fn hit(&self, x: f32, y: f32) -> Option<Target> {
        self.hit_rect(x, y).map(|(_, t)| t)
    }

    fn hit_rect(&self, x: f32, y: f32) -> Option<(Rect, Target)> {
        // Last drawn wins: overlays are drawn after the rails.
        self.hits.borrow().iter().rev().find(|(r, _)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h).copied()
    }

    /// Moves focus to whatever the pointer is over. Returns true when something changed.
    pub fn pointer(&mut self, x: f32, y: f32) -> bool {
        let Some((rect, target)) = self.hit_rect(x, y) else { return false };
        if matches!(self.mode, Mode::Homebrew(_)) {
            let changed = self.brew_pointer(target);
            if changed {
                self.refocus(false);
            }
            return changed;
        }
        if self.look == Look::Glass {
            // The tile under the pointer tilts towards it.
            self.tilt_x.target = ((x - rect.x - rect.w * 0.5) / (rect.w * 0.5)).clamp(-1.0, 1.0);
            self.tilt_y.target = ((y - rect.y - rect.h * 0.5) / (rect.h * 0.5)).clamp(-1.0, 1.0);
        }
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

    pub fn set_apps(&mut self, mut apps: Vec<App>) {
        // The Homebrew page opens from a tile among the apps (it can be moved and hidden).
        apps.push(App {
            id: homebrew::HOMEBREW_TILE.into(),
            title: "Homebrew".into(),
            icon_path: None,
            bg_color: Some("#C0392B".into()),
            params: None,
            art_path: None,
            art_is_splash: false,
        });
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
                Service::Wallpaper => status.wallpaper_version != self.status.wallpaper_version,
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
            .or(status.wallpaper_setup.as_ref())
            .and_then(|url| qrcodegen::QrCode::encode_text(url, qrcodegen::QrCodeEcc::Medium).ok().map(|qr| (url.clone(), qr)));
        // A new photo from the phone becomes the wallpaper.
        if status.wallpaper_version != self.status.wallpaper_version {
            self.prefs.ui.wallpaper = "custom".into();
            self.pending.push(Action::Wallpaper("custom".into()));
            self.pending.push(Action::SavePrefs(self.prefs.clone()));
        }
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
        let channel = icon.app_id == crate::model::HOMEBREW_CHANNEL;
        self.add_icon_for(icon, renderer);
        // The Homebrew tile wears Homebrew Channel's icon.
        if channel {
            self.alias(crate::model::HOMEBREW_CHANNEL, homebrew::HOMEBREW_TILE);
        }
    }

    fn alias(&mut self, from: &str, to: &str) {
        if let Some(icon) = self.icons.get(from).copied() {
            self.icons.insert(to.to_string(), icon);
        }
        if let Some(fill) = self.blended.get(from).copied() {
            self.blended.insert(to.to_string(), fill);
        }
        if let Some(color) = self.colors.get(from).copied() {
            self.tint_id(to, color);
        }
    }

    fn add_icon_for(&mut self, icon: Icon, renderer: &Renderer) {
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
            Mode::Move { .. } if self.look == Look::Glass => self.glass_move_key(key),
            Mode::Move { .. } => self.move_key(key),
            Mode::Setup { .. } => self.setup_key(key),
            Mode::Offer { .. } => self.offer_key(key),
            Mode::Picker(_) => self.picker_key(key),
            Mode::Settings => self.settings_key(key),
            Mode::Store(_) => self.store_key(key),
            Mode::Homebrew(_) => self.homebrew_key(key),
            Mode::Browse if self.header && self.look == Look::Cinematic => self.header_key(key),
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
        let mut appearance = vec![item(SettingId::Theme, s.theme, self.look.name().to_string(), None)];
        if self.look == Look::Glass {
            appearance.push(item(SettingId::Wallpaper, s.wallpaper, self.wallpaper_name(&ui.wallpaper).to_string(), None));
            appearance.push(item(SettingId::WallpaperUpload, s.wallpaper_upload, String::new(), None));
        }
        // Ambient light belongs to Cinematic (Glass has its wallpaper).
        if self.look == Look::Cinematic {
            appearance.push(item(SettingId::Ambient, s.ambient_light, s.levels[ui.ambient.min(3) as usize].to_string(), None));
        }
        appearance.extend([
            item(SettingId::Motion, s.motion, if ui.reduced_motion { s.reduced } else { s.full }.to_string(), None),
            item(SettingId::Preview, s.live_preview, onoff(ui.preview), Some(ui.preview)),
        ]);
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
            SettingId::Wallpaper => {
                let names = self.wallpapers();
                let at = names.iter().position(|n| *n == self.prefs.ui.wallpaper).unwrap_or(0);
                let next = names[(at + 1) % names.len()];
                self.prefs.ui.wallpaper = next.to_string();
                self.pending.push(Action::Wallpaper(next.to_string()));
                return Some(Action::SavePrefs(self.prefs.clone()));
            }
            SettingId::WallpaperUpload => {
                self.mode = Mode::Setup { service: Service::Wallpaper, closing_at: None };
                self.from_settings = true;
                return Some(Action::Service(ServiceRequest::WallpaperSetup(true)));
            }
            SettingId::Theme => {
                let at = Look::ALL.iter().position(|&l| l == self.look).unwrap_or(0);
                let next = Look::ALL[(at + 1) % Look::ALL.len()];
                ui.theme = next.id().to_string();
                self.pending.push(Action::SetTheme(next));
            }
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

    pub fn look(&self) -> Look {
        self.look
    }

    fn browse_key(&mut self, key: Key, home_mode: bool) -> Option<Action> {
        match self.look {
            Look::Cinematic => self.cinematic_browse_key(key, home_mode),
            Look::Glass => self.glass_browse_key(key, home_mode),
        }
    }

    fn cinematic_browse_key(&mut self, key: Key, home_mode: bool) -> Option<Action> {
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
        if self.focused().is_some_and(|t| t.id == homebrew::HOMEBREW_TILE) {
            return self.open_homebrew();
        }
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
            Service::Wallpaper => false,
            Service::TiviOne | Service::HomeTiles => false,
        };
        match key {
            Key::Back => {
                self.mode = self.after_overlay();
                match (service, connected) {
                    (Service::HomeAssistant, false) => Some(Action::Service(ServiceRequest::HaSetup(false))),
                    (Service::Plex, false) => Some(Action::Service(ServiceRequest::PlexLink(false))),
                    (Service::Weather, _) => Some(Action::Service(ServiceRequest::LocationSetup(false))),
                    (Service::Wallpaper, _) => Some(Action::Service(ServiceRequest::WallpaperSetup(false))),
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

    /// Where the hold-OK menu opens: the focused tile's x and the y it sits above.
    fn menu_anchor(&self) -> (f32, f32) {
        match self.look {
            Look::Cinematic => {
                let rail = &self.rails[self.active];
                let title_y = RAIL_TITLE_Y + self.rail_slot(self.active) as f32 * RAIL_PITCH + self.vscroll.value;
                (rail.slot_x(rail.focus as f32) - rail.scroll.value, title_y)
            }
            Look::Glass => self.glass_menu_anchor(),
        }
    }

    fn focused(&self) -> Option<&Tile> {
        let rail = &self.rails[self.active];
        rail.tiles.get(rail.focus)
    }

    // ---- Animation ------------------------------------------------------------------

    fn refocus(&mut self, instant: bool) {
        let moving = matches!(self.mode, Mode::Move { .. });
        let overlay = matches!(self.mode, Mode::Menu(_) | Mode::Setup { .. } | Mode::Offer { .. } | Mode::Picker(_) | Mode::Settings | Mode::Store(_) | Mode::Homebrew(_));
        self.brew_refocus();
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
        match self.look {
            Look::Cinematic => self.vscroll.target = -(self.rail_slot(active) as f32) * RAIL_PITCH,
            Look::Glass => self.glass_refocus(instant),
        }

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
        moving |= self.brew_scroll.step(dt);
        if self.look == Look::Glass {
            moving |= self.glass_step(dt);
        }
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
                tile.gcol.snap();
                tile.grow.snap();
            }
        }
        for spring in [&mut self.shelf, &mut self.shelf_scroll, &mut self.settings_focus, &mut self.backdrop_t] {
            spring.snap();
        }
        self.vscroll.snap();
        self.hero_t.snap();
        self.overlay.snap();
        self.lift.snap();
        self.previous_hero = None;
    }

    // ---- Drawing --------------------------------------------------------------------

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
