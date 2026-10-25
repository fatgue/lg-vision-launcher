//! Persistent data outside the app payload, so it survives reinstalls:
//! `/media/developer/vision` on the TV, the user's config dir on desktop.
//!   prefs.json        favourites order and hidden apps
//!   icons/<id>.icon   decoded icon cache (RGBA, ≤128 px, dominant colour)
//!   art/<id>.icon     decoded tile artwork (RGBA 320×180, dominant colour)
use crate::model::Icon;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const ICON_MAGIC: &[u8; 4] = b"VIC2";

pub fn data_dir(webos: bool) -> PathBuf {
    if webos {
        return PathBuf::from("/media/developer/vision");
    }
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("vision")
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Prefs {
    /// Favourites in the user's order; apps not listed follow in LG's order.
    pub order: Vec<String>,
    pub hidden: Vec<String>,
    pub ui: UiPrefs,
}

/// Choices from the Settings panel.
#[derive(Clone, Debug, PartialEq)]
pub struct UiPrefs {
    pub weather: bool,
    pub fahrenheit: bool,
    pub clock24: bool,
    pub show_date: bool,
    /// Rows the user switched off: "continue", "tv", "home", "inputs", "hidden".
    pub hidden_rows: Vec<String>,
    /// Ambient light: 0 off, 1 low, 2 normal, 3 high.
    pub ambient: u8,
    pub reduced_motion: bool,
    /// Live preview of TV channels in the hero.
    pub preview: bool,
    /// UI language code; empty follows the TV.
    pub language: String,
    /// The TiviOne tile on Home while TiviOne is not installed (the user can hide it).
    pub tivione_promo: bool,
    /// The look (`theme::Look::id`).
    pub theme: String,
    /// Glass wallpaper: a bundled one (`valley`, `crete`, `dunes`) or `custom` (from the phone).
    pub wallpaper: String,
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            weather: true,
            fahrenheit: false,
            clock24: true,
            show_date: true,
            hidden_rows: Vec::new(),
            ambient: 2,
            reduced_motion: false,
            preview: true,
            language: String::new(),
            tivione_promo: true,
            theme: "cinematic".into(),
            wallpaper: "valley".into(),
        }
    }
}

impl Prefs {
    pub fn load(dir: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(dir.join("prefs.json")) else {
            return Self::default();
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            return Self::default();
        };
        let list = |key: &str| -> Vec<String> {
            value
                .get(key)
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default()
        };
        let ui = value.get("ui");
        let flag = |key: &str, default: bool| ui.and_then(|u| u.get(key)).and_then(Value::as_bool).unwrap_or(default);
        let defaults = UiPrefs::default();
        let ui = UiPrefs {
            weather: flag("weather", defaults.weather),
            fahrenheit: flag("fahrenheit", defaults.fahrenheit),
            clock24: flag("clock24", defaults.clock24),
            show_date: flag("showDate", defaults.show_date),
            hidden_rows: ui
                .and_then(|u| u.get("hiddenRows"))
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
            ambient: ui.and_then(|u| u.get("ambient")).and_then(Value::as_u64).map_or(defaults.ambient, |a| a.min(3) as u8),
            reduced_motion: flag("reducedMotion", defaults.reduced_motion),
            preview: flag("preview", defaults.preview),
            language: ui.and_then(|u| u.get("language")).and_then(Value::as_str).unwrap_or_default().to_string(),
            tivione_promo: flag("tivionePromo", defaults.tivione_promo),
            theme: ui.and_then(|u| u.get("theme")).and_then(Value::as_str).unwrap_or(&defaults.theme).to_string(),
            wallpaper: ui.and_then(|u| u.get("wallpaper")).and_then(Value::as_str).unwrap_or(&defaults.wallpaper).to_string(),
        };
        Self { order: list("order"), hidden: list("hidden"), ui }
    }

    /// Atomic write (temp file + rename) so a power cut never leaves half a file.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let temp = dir.join("prefs.json.tmp");
        let ui = &self.ui;
        let body = json!({
            "version": 1,
            "order": self.order,
            "hidden": self.hidden,
            "ui": {
                "weather": ui.weather,
                "fahrenheit": ui.fahrenheit,
                "clock24": ui.clock24,
                "showDate": ui.show_date,
                "hiddenRows": ui.hidden_rows,
                "ambient": ui.ambient,
                "reducedMotion": ui.reduced_motion,
                "preview": ui.preview,
                "language": ui.language,
                "tivionePromo": ui.tivione_promo,
                "theme": ui.theme,
                "wallpaper": ui.wallpaper,
            },
        });
        std::fs::write(&temp, serde_json::to_vec_pretty(&body).unwrap_or_default())?;
        std::fs::rename(temp, dir.join("prefs.json"))
    }
}

fn icon_file(dir: &Path, kind: &str, app_id: &str) -> PathBuf {
    let safe: String = app_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' })
        .collect();
    dir.join(kind).join(format!("{safe}.icon"))
}

/// Returns the cached image (`kind` = "icons" or "art") if it was decoded from the same source.
pub fn load_icon(dir: &Path, kind: &str, app_id: &str, source: &str) -> Option<Icon> {
    let bytes = std::fs::read(icon_file(dir, kind, app_id)).ok()?;
    let (header, rest) = bytes.split_at_checked(16)?;
    if &header[0..4] != ICON_MAGIC {
        return None;
    }
    let w = u16::from_le_bytes([header[4], header[5]]) as u32;
    let h = u16::from_le_bytes([header[6], header[7]]) as u32;
    let color = (header[8] != 0).then(|| [header[9], header[10], header[11]]);
    let path_len = u32::from_le_bytes([header[12], header[13], header[14], header[15]]) as usize;
    let (path, pixels) = rest.split_at_checked(path_len)?;
    if path != source.as_bytes() || pixels.len() != (w * h * 4) as usize {
        return None;
    }
    Some(Icon { app_id: app_id.to_string(), w, h, pixels: pixels.to_vec(), color })
}

pub fn save_icon(dir: &Path, kind: &str, icon: &Icon, source: &str) -> std::io::Result<()> {
    let file = icon_file(dir, kind, &icon.app_id);
    std::fs::create_dir_all(file.parent().unwrap())?;
    let mut bytes = Vec::with_capacity(16 + source.len() + icon.pixels.len());
    bytes.extend_from_slice(ICON_MAGIC);
    bytes.extend_from_slice(&(icon.w as u16).to_le_bytes());
    bytes.extend_from_slice(&(icon.h as u16).to_le_bytes());
    let color = icon.color.unwrap_or([0; 3]);
    bytes.extend_from_slice(&[icon.color.is_some() as u8, color[0], color[1], color[2]]);
    bytes.extend_from_slice(&(source.len() as u32).to_le_bytes());
    bytes.extend_from_slice(source.as_bytes());
    bytes.extend_from_slice(&icon.pixels);
    let temp = file.with_extension("tmp");
    std::fs::write(&temp, bytes)?;
    std::fs::rename(temp, file)
}

/// Remembers that a source yields no usable image (e.g. a splash without a logo), so it is
/// not decoded again on every start.
pub fn mark_unusable(dir: &Path, kind: &str, app_id: &str, source: &str) {
    let file = icon_file(dir, kind, app_id).with_extension("none");
    let _ = std::fs::create_dir_all(file.parent().unwrap());
    let _ = std::fs::write(file, source);
}

pub fn is_unusable(dir: &Path, kind: &str, app_id: &str, source: &str) -> bool {
    std::fs::read_to_string(icon_file(dir, kind, app_id).with_extension("none")).is_ok_and(|s| s == source)
}
