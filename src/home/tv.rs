//! The TV section of Settings: the TV's own settings, read and changed through webOS's settings
//! service. They load when the section gets focus (on the platform thread) and cost nothing while
//! drawing. Picture settings apply to the current input and signal, as in LG's menu.
use super::*;
use crate::model::{TvControl, TvSettings};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Control {
    Toggle,
    /// One of the values the TV allows now (OK picks the next).
    List,
    /// 0–100 (Left/Right change it).
    Range,
    /// LG's own settings app.
    Open,
}

/// The rows, by "category.key".
pub(super) const TV_ROWS: [(&str, Control); 15] = [
    ("general.homeAutoLaunch", Control::Toggle),
    ("option.quickStartMode", Control::Toggle),
    ("picture.pictureMode", Control::List),
    ("picture.backlight", Control::Range),
    ("picture.contrast", Control::Range),
    ("picture.energySaving", Control::List),
    ("other.uhdDeepColorHDMI1", Control::Toggle),
    ("other.uhdDeepColorHDMI2", Control::Toggle),
    ("other.uhdDeepColorHDMI3", Control::Toggle),
    ("other.uhdDeepColorHDMI4", Control::Toggle),
    ("sound.soundMode", Control::List),
    ("sound.soundOutput", Control::List),
    ("time.sleepTimer", Control::List),
    ("time.automaticStandBy", Control::List),
    ("", Control::Open),
];

/// LG's settings app.
const LG_SETTINGS: &str = "com.palm.app.settings";

impl Home {
    /// The TV section's rows: those the TV reports, then LG's settings.
    pub(super) fn tv_items(&self) -> Vec<SettingItem> {
        let s = self.strings;
        let mut items = Vec::new();
        let Some(tv) = &self.tv_settings else {
            let info = SettingItem { id: SettingId::Info, label: s.loading.to_string(), value: String::new(), on: None, enabled: false };
            items.push(info);
            items.push(SettingItem { id: SettingId::Tv(14), label: s.tv_labels[9].to_string(), value: String::new(), on: None, enabled: true });
            return items;
        };
        for (i, &(key, control)) in TV_ROWS.iter().enumerate().take(14) {
            let Some(value) = tv.values.get(key) else { continue };
            let label = match i {
                0 | 1 => s.tv_labels[i].to_string(),
                2 => s.tv_labels[2].to_string(),
                3 => s.brightness.to_string(),
                4 => s.tv_labels[3].to_string(),
                5 => s.tv_labels[4].to_string(),
                6..=9 => format!("{} · HDMI {}", s.tv_labels[5], i - 5),
                10 => s.tv_labels[6].to_string(),
                11 => s.tv_labels[7].to_string(),
                12 => s.sleep_timer.to_string(),
                _ => s.tv_labels[8].to_string(),
            };
            let on = (control == Control::Toggle).then(|| value == "on");
            // A switch shows its own state.
            let value = if on.is_some() { String::new() } else { self.tv_value(key, value) };
            items.push(SettingItem { id: SettingId::Tv(i), label, value, on, enabled: true });
        }
        items.push(SettingItem { id: SettingId::Tv(14), label: s.tv_labels[9].to_string(), value: String::new(), on: None, enabled: true });
        items
    }

    /// A value as the TV's menu would name it.
    pub(super) fn tv_value(&self, key: &str, value: &str) -> String {
        let s = self.strings;
        match key {
            "picture.pictureMode" => crate::strings::picture_mode_name(value),
            "sound.soundMode" => crate::strings::sound_mode_name(value),
            "picture.energySaving" => match value {
                "auto" => s.energy_levels[0].into(),
                "off" => s.off.into(),
                "min" => s.energy_levels[1].into(),
                "med" => s.energy_levels[2].into(),
                "max" => s.energy_levels[3].into(),
                "screen_off" => s.energy_levels[4].into(),
                other => other.into(),
            },
            "sound.soundOutput" => match value {
                "tv_speaker" | "tv_external_speaker" => s.tv_speaker.into(),
                "external_optical" => "Optical".into(),
                "external_arc" => "HDMI ARC".into(),
                "bt_soundbar" | "bt_audio" => "Bluetooth".into(),
                "headphone" => s.headphones.into(),
                "wisa_speaker" => "WiSA".into(),
                "lineout" => "Line out".into(),
                "tv_speaker_optical_arc" => format!("{} + Optical", s.tv_speaker),
                other => other.replace('_', " "),
            },
            "time.sleepTimer" => match value {
                "off" => s.off.into(),
                minutes => s.minutes.replace("{n}", minutes),
            },
            "time.automaticStandBy" => match value {
                "0" | "off" => s.off.into(),
                hours => s.hours.replace("{n}", hours),
            },
            _ => value.to_string(),
        }
    }

    /// OK on a TV row: flip a switch, pick the next value, or open LG's settings.
    pub(super) fn tv_change(&mut self, row: usize) -> Option<Action> {
        let (key, control) = *TV_ROWS.get(row)?;
        if control == Control::Open {
            self.mode = Mode::Browse;
            return Some(Action::Launch { app_id: LG_SETTINGS.into(), params: None });
        }
        let tv = self.tv_settings.as_mut()?;
        let current = tv.values.get(key)?.clone();
        let value = match control {
            Control::Toggle => if current == "on" { "off" } else { "on" }.to_string(),
            Control::List => {
                let options = tv.options.get(key).filter(|o| !o.is_empty())?;
                let at = options.iter().position(|o| *o == current).map_or(0, |i| i + 1);
                options[at % options.len()].clone()
            }
            Control::Range | Control::Open => return None,
        };
        Some(set(tv, key, value))
    }

    /// Left/Right on a range row.
    pub(super) fn tv_adjust(&mut self, row: usize, step: i32) -> Option<Action> {
        let (key, control) = *TV_ROWS.get(row)?;
        if control != Control::Range {
            return None;
        }
        let tv = self.tv_settings.as_mut()?;
        let current: i32 = tv.values.get(key)?.parse().ok()?;
        Some(set(tv, key, (current + step).clamp(0, 100).to_string()))
    }

    /// Whether Left/Right on this settings row changes a value instead of leaving the list.
    pub(super) fn tv_range(id: SettingId) -> Option<usize> {
        match id {
            SettingId::Tv(row) if TV_ROWS.get(row).is_some_and(|r| r.1 == Control::Range) => Some(row),
            _ => None,
        }
    }
}

/// Shows the new value at once; the TV's answer follows.
pub(super) fn set(tv: &mut TvSettings, key: &str, value: String) -> Action {
    tv.values.insert(key.to_string(), value.clone());
    let (category, name) = key.split_once('.').unwrap_or((key, ""));
    Action::TvControl(TvControl::Set { category: category.into(), key: name.into(), value })
}
