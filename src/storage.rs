//! Persistent data outside the app payload, so it survives reinstalls:
//! `/media/developer/vision` on the TV, the user's config dir on desktop.
//!   prefs.json        favourites order and hidden apps
//!   icons/<id>.icon   decoded icon cache (RGBA, ≤128 px, dominant colour)
use crate::model::Icon;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const ICON_MAGIC: &[u8; 4] = b"VIC1";

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
        Self { order: list("order"), hidden: list("hidden") }
    }

    /// Atomic write (temp file + rename) so a power cut never leaves half a file.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let temp = dir.join("prefs.json.tmp");
        let body = json!({ "version": 1, "order": self.order, "hidden": self.hidden });
        std::fs::write(&temp, serde_json::to_vec_pretty(&body).unwrap_or_default())?;
        std::fs::rename(temp, dir.join("prefs.json"))
    }
}

fn icon_file(dir: &Path, app_id: &str) -> PathBuf {
    let safe: String = app_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' })
        .collect();
    dir.join("icons").join(format!("{safe}.icon"))
}

/// Returns the cached icon if it was decoded from the same source path.
pub fn load_icon(dir: &Path, app_id: &str, source: &str) -> Option<Icon> {
    let bytes = std::fs::read(icon_file(dir, app_id)).ok()?;
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

pub fn save_icon(dir: &Path, icon: &Icon, source: &str) -> std::io::Result<()> {
    let file = icon_file(dir, &icon.app_id);
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
