//! Design tokens for direction A, "Cinematic Ambient".
use crate::color::Rgba;
use crate::strings::Strings;
use crate::text::{Style, Weight};

/// The overall look of the home screen: layout, navigation, drawing and fonts, not only colours.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Look {
    /// Hero text and horizontal rows over ambient light (Vision's original design).
    Cinematic,
    /// Top Shelf and a five-column app grid, inspired by tvOS.
    Glass,
}

impl Look {
    pub const ALL: [Look; 2] = [Look::Cinematic, Look::Glass];

    /// Stable name in prefs.json and on the command line (`--look`).
    pub fn id(self) -> &'static str {
        match self {
            Look::Cinematic => "cinematic",
            Look::Glass => "glass",
        }
    }

    pub fn from_id(id: &str) -> Self {
        Look::ALL.into_iter().find(|look| look.id() == id).unwrap_or(Look::Cinematic)
    }

    /// Shown in Settings (a name, not translated).
    pub fn name(self) -> &'static str {
        match self {
            Look::Cinematic => "Cinematic",
            Look::Glass => "Glass",
        }
    }

    /// Regular, Medium and SemiBold, in `assets/fonts`.
    pub fn fonts(self) -> [&'static str; 3] {
        match self {
            Look::Cinematic => ["Outfit-Regular.ttf", "Outfit-Medium.ttf", "Outfit-SemiBold.ttf"],
            Look::Glass => ["Inter-Regular.ttf", "Inter-Medium.ttf", "Inter-SemiBold.ttf"],
        }
    }

    /// Each look keeps its own packed glyph atlas, so switching back is fast.
    pub fn glyph_cache(self) -> String {
        format!("glyphs-{}.cache", self.id())
    }

    pub fn background(self) -> Rgba {
        match self {
            Look::Cinematic => BG,
            Look::Glass => GLASS_BG,
        }
    }

    pub fn prewarm_jobs(self, strings: &Strings) -> Vec<(Style, String)> {
        let mut jobs = prewarm_jobs(strings);
        if self == Look::Glass {
            let latin: String = (' '..='~').collect();
            let ui = format!("{}{}", crate::strings::charset(strings), "0123456789:, ");
            for style in [GLASS_LABEL, GLASS_SECTION, GLASS_CLOCK] {
                jobs.push((style, latin.clone() + &ui));
            }
        }
        jobs
    }
}

// Glass (inspired by tvOS): near-black, white text, small radii, no focus ring.
pub const GLASS_BG: Rgba = Rgba::hex(0x0B0B0F);
pub const GLASS_LABEL: Style = Style::new(Weight::SemiBold, 26.0);
pub const GLASS_SECTION: Style = Style::new(Weight::SemiBold, 30.0);
pub const GLASS_CLOCK: Style = Style::new(Weight::SemiBold, 46.0);

pub const BG: Rgba = Rgba::hex(0x07080B);
pub const TEXT: Rgba = Rgba::hex(0xF5F1E8);
pub const ACCENT_DEFAULT: Rgba = Rgba::hex(0xE9A23B);
pub const ACCENT_INPUT: Rgba = Rgba::hex(0x3E7BFA);
pub const CONNECTED: Rgba = Rgba::hex(0x4ADE80);
pub const TILE_NEUTRAL: Rgba = Rgba::hex(0x1E1E22);

pub const SAFE_X: f32 = 96.0;
pub const SAFE_TOP: f32 = 54.0;

pub const TILE_RADIUS: f32 = 22.0;
pub const TILE_GAP: f32 = 28.0;
pub const FOCUS_SCALE: f32 = 1.1;
pub const RING_WIDTH: f32 = 4.0;
pub const RING_OFFSET: f32 = 6.0;

pub const BRAND: Style = Style::new(Weight::SemiBold, 28.0).tracking(0.02);
pub const HEADER: Style = Style::new(Weight::Regular, 26.0);
pub const CLOCK: Style = Style::new(Weight::Medium, 40.0);
pub const EYEBROW: Style = Style::new(Weight::Medium, 24.0).tracking(0.14);
pub const HERO_SIZES: [f32; 4] = [112.0, 96.0, 80.0, 64.0];
pub const HERO_SUB: Style = Style::new(Weight::Regular, 30.0);
pub const RAIL_TITLE: Style = Style::new(Weight::Medium, 26.0);
pub const TILE_LABEL: Style = Style::new(Weight::SemiBold, 24.0);
pub const MONOGRAM: Style = Style::new(Weight::SemiBold, 34.0);
pub const INPUT_LABEL: Style = Style::new(Weight::SemiBold, 26.0);
pub const INPUT_SUB: Style = Style::new(Weight::Regular, 20.0);
pub const NOTICE: Style = Style::new(Weight::Regular, 22.0);
pub const HINT: Style = Style::new(Weight::Regular, 24.0);
pub const MENU_TITLE: Style = Style::new(Weight::Medium, 24.0);
pub const MENU_ITEM: Style = Style::new(Weight::Medium, 28.0);
pub const ENTITY_NAME: Style = Style::new(Weight::SemiBold, 26.0);
pub const SETUP_TITLE: Style = Style::new(Weight::SemiBold, 52.0).tracking(-0.01);
pub const SETUP_BODY: Style = Style::new(Weight::Regular, 30.0);
pub const SETUP_SMALL: Style = Style::new(Weight::Regular, 22.0);
pub const DIGITS: Style = Style::new(Weight::SemiBold, 44.0).tracking(0.08);
pub const SETUP_CODE: Style = Style::new(Weight::SemiBold, 150.0).tracking(0.12);

pub fn hero(size: f32) -> Style {
    Style::new(Weight::SemiBold, size).tracking(-0.02)
}

/// Glyphs to rasterise at startup, per style: everything the fixed UI draws, plus Latin
/// text for app titles. Rare sizes and scripts still rasterise lazily.
pub fn prewarm_jobs(strings: &Strings) -> Vec<(Style, String)> {
    let latin: String = (' '..='~').chain("\u{e4}\u{f6}\u{fc}\u{c4}\u{d6}\u{dc}\u{df}\u{e9}\u{e8}\u{e1}\u{e0}\u{e7}\u{f1}\u{b7}\u{2013}\u{2014}\u{2019}\u{201c}\u{201d}\u{2026}&".chars()).collect();
    let ui = format!("{}{}", crate::strings::charset(strings), "0123456789:, ");
    let upper = latin.to_uppercase() + &ui.to_uppercase();
    let mut jobs = vec![
        (BRAND, "Vision".to_string()),
        (CLOCK, "0123456789:".to_string()),
        (HEADER, ui.clone()),
        (EYEBROW, upper),
        (HERO_SUB, ui.clone()),
        (HINT, ui.clone()),
        (RAIL_TITLE, ui.clone()),
        (INPUT_SUB, ui.clone()),
        (NOTICE, latin.clone() + &ui),
        (MENU_ITEM, ui.clone()),
        (MENU_TITLE, latin.clone()),
        (TILE_LABEL, latin.clone()),
        (MONOGRAM, latin.clone()),
        (INPUT_LABEL, latin.clone()),
        (ENTITY_NAME, latin.clone()),
        (SETUP_TITLE, ui.clone()),
        (SETUP_BODY, latin.clone() + &ui),
        (SETUP_SMALL, latin.clone() + &ui),
        (SETUP_CODE, "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789".to_string()),
        (DIGITS, "0123456789_ ".to_string()),
    ];
    for size in &HERO_SIZES[..2] {
        jobs.push((hero(*size), latin.clone()));
    }
    jobs
}
