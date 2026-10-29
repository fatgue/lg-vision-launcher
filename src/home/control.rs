//! Glass: the two round buttons at the top right (Inputs and the Control Center), the inputs
//! list and the Control Center: the TV's sound and picture mode, the look, the sleep timer,
//! screen off, power, the wallpaper, Settings and the brightness.
use super::glass::glass_surface;
use super::*;
use crate::model::{TvControl, TvSettings};

/// Round buttons at the top right: their centres (Inputs, Control Center) and diameter.
pub(super) const BUTTONS: [(f32, f32); 2] = [(1664.0, 100.0), (1750.0, 100.0)];
pub(super) const BUTTON_D: f32 = 72.0;

/// The Control Center's area (top right, under the buttons) and its items.
const PANEL_X: f32 = 1225.0;
const PANEL_Y: f32 = 160.0;
const PANEL_W: f32 = 560.0;
const PILL: (f32, f32) = (272.0, 100.0);
const CIRCLE: f32 = 100.0;
const ITEM_GAP: f32 = 16.0;

#[derive(Clone, Copy, PartialEq)]
enum Item {
    Sound,
    Picture,
    Theme,
    Sleep,
    ScreenOff,
    Power,
    Wallpaper,
    Settings,
    Brightness,
}

/// Rows of the Control Center, top to bottom.
const ROWS: [&[Item]; 4] = [
    &[Item::Sound, Item::Picture],
    &[Item::Theme, Item::Sleep],
    &[Item::ScreenOff, Item::Power, Item::Wallpaper, Item::Settings],
    &[Item::Brightness],
];

const SLEEP: [&str; 9] = ["off", "10", "20", "30", "60", "90", "120", "180", "240"];

fn item_at(index: usize) -> (usize, usize, Item) {
    let mut i = index;
    for (row, items) in ROWS.iter().enumerate() {
        if i < items.len() {
            return (row, i, items[i]);
        }
        i -= items.len();
    }
    (ROWS.len() - 1, 0, Item::Brightness)
}

fn index_of(row: usize, column: usize) -> usize {
    ROWS[..row].iter().map(|r| r.len()).sum::<usize>() + column.min(ROWS[row].len() - 1)
}

fn item_rect(row: usize, column: usize) -> Rect {
    let y = PANEL_Y + row as f32 * (PILL.1 + ITEM_GAP);
    match ROWS[row].len() {
        1 => Rect::new(PANEL_X, y, PANEL_W, PILL.1),
        2 => Rect::new(PANEL_X + column as f32 * (PILL.0 + ITEM_GAP), y, PILL.0, PILL.1),
        n => {
            let gap = (PANEL_W - n as f32 * CIRCLE) / (n - 1) as f32;
            Rect::new(PANEL_X + column as f32 * (CIRCLE + gap), y, CIRCLE, CIRCLE)
        }
    }
}

impl Home {
    // ---- The buttons ------------------------------------------------------------------

    pub(super) fn draw_header_buttons(&self, r: &mut Renderer, text: &mut Text, fade: f32) {
        let f = self.settings_focus.value.clamp(0.0, 1.0);
        let browse = matches!(self.mode, Mode::Browse);
        for (i, &(cx, cy)) in BUTTONS.iter().enumerate() {
            let rect = Rect::new(cx - BUTTON_D * 0.5, cy - BUTTON_D * 0.5, BUTTON_D, BUTTON_D);
            if browse {
                self.hits.borrow_mut().push((rect, Target::HeaderButton(i)));
            }
            let lit = if i == self.header_item { f } else { 0.0 } * fade;
            let rect = rect.scale_about(1.0 + 0.08 * lit, 0.5, 0.5);
            glass_surface(r, self.wallpaper, self.backdrop_t.value, rect, BUTTON_D * 0.5, fade);
            if lit > 0.01 {
                r.rect(rect, BUTTON_D * 0.5, Rgba::hex(0xFFFFFF).alpha(0.92 * lit));
            }
            let ch = if i == 0 { icon::INPUTS } else { icon::CONTROLS };
            let color = Rgba::hex(0xFFFFFF).lerp(Rgba::hex(0x1C1C1E), lit);
            text.icon(r, ch, cx, cy, ICON_SMALL, color.alpha(fade));
        }
    }

    /// OK on a button: the inputs list or the Control Center.
    pub(super) fn open_header_item(&mut self) -> Option<Action> {
        if self.header_item == 0 {
            self.mode = Mode::Inputs(0);
            None
        } else {
            self.mode = Mode::Control(0);
            Some(Action::TvSettings)
        }
    }

    pub fn set_tv_settings(&mut self, settings: Option<TvSettings>) {
        self.tv_settings = settings;
    }

    // ---- Inputs -----------------------------------------------------------------------

    /// Live TV, then the inputs: (title, app to launch, connected, device name).
    fn input_list(&self) -> Vec<(String, String, Option<bool>, String)> {
        let mut list = vec![(self.strings.live_tv.to_string(), "com.webos.app.livetv".to_string(), None, String::new())];
        for tile in &self.rails[INPUTS].tiles {
            let connected = match tile.kind {
                Kind::Input { connected } => Some(connected),
                _ => None,
            };
            list.push((tile.title.clone(), tile.launch_id.clone(), connected, tile.detail.clone()));
        }
        list
    }

    pub(super) fn inputs_key(&mut self, key: Key) -> Option<Action> {
        let Mode::Inputs(focus) = self.mode else { return None };
        let list = self.input_list();
        match key {
            Key::Up if focus > 0 => self.mode = Mode::Inputs(focus - 1),
            Key::Down if focus + 1 < list.len() => self.mode = Mode::Inputs(focus + 1),
            Key::Ok => {
                let (_, app, _, _) = list.get(focus)?.clone();
                self.mode = Mode::Browse;
                self.header = false;
                return Some(Action::Launch { app_id: app, params: None });
            }
            Key::Back | Key::Left | Key::Right => self.mode = Mode::Browse,
            _ => {}
        }
        None
    }

    pub(super) fn draw_inputs(&self, r: &mut Renderer, text: &mut Text) {
        let Mode::Inputs(focus) = self.mode else { return };
        let open = self.overlay.value.clamp(0.0, 1.0);
        let list = self.input_list();
        let (row_h, w) = (76.0, 460.0);
        let x = BUTTONS[0].0 + BUTTON_D * 0.5 - w;
        let panel = Rect::new(x, PANEL_Y - 10.0 * (1.0 - open), w, 84.0 + list.len() as f32 * row_h);
        r.rect(Rect::new(0.0, 0.0, LOGICAL_W, LOGICAL_H), 0.0, Rgba::hex(0).alpha(0.3 * open));
        r.shadow(panel.offset(0.0, 16.0), 36.0, 40.0, Rgba::hex(0).alpha(0.35 * open));
        glass_surface(r, self.wallpaper, self.backdrop_t.value, panel, 36.0, open);
        let white = Rgba::hex(0xFFFFFF);
        text.draw_top(r, self.strings.inputs, panel.x + 32.0, panel.y + 22.0, GLASS_SMALL, white.alpha(0.7 * open));
        for (i, (title, _, connected, device)) in list.iter().enumerate() {
            let row = Rect::new(panel.x + 12.0, panel.y + 64.0 + i as f32 * row_h, w - 24.0, row_h - 8.0);
            self.hits.borrow_mut().push((row, Target::InputRow(i)));
            let focused = i == focus;
            if focused {
                r.rect(row, row.h * 0.5, white.alpha(0.92 * open));
            }
            let color = if focused { Rgba::hex(0x1C1C1E) } else { white };
            let ch = if i == 0 { icon::PICTURE } else { icon::INPUTS };
            text.icon(r, ch, row.x + 40.0, row.y + row.h * 0.5, ICON_SMALL, color.alpha(open));
            let title_w = text.draw(r, title, row.x + 76.0, row.y + row.h * 0.5 + 11.0, GLASS_ROW, color.alpha(open));
            if !device.is_empty() {
                let device = clip(text, r, device, GLASS_SMALL, row.w - 76.0 - title_w - 70.0);
                text.draw(r, &device, row.x + 76.0 + title_w + 14.0, row.y + row.h * 0.5 + 9.0, GLASS_SMALL, color.alpha(0.6 * open));
            }
            if let Some(on) = connected {
                let dot = if *on { Rgba::hex(0x30D158) } else { color.alpha(0.3) };
                r.rect(Rect::new(row.x + row.w - 36.0, row.y + row.h * 0.5 - 7.0, 14.0, 14.0), 7.0, dot.fade(open));
            }
        }
    }

    // ---- Control Center ---------------------------------------------------------------

    pub(super) fn control_key(&mut self, key: Key) -> Option<Action> {
        let Mode::Control(focus) = self.mode else { return None };
        let (row, column, item) = item_at(focus);
        // Up/Down keep the nearest column; Left/Right stay in the row (and set the brightness).
        let nearest = |to: usize| {
            let x = item_rect(row, column).x + item_rect(row, column).w * 0.5;
            (0..ROWS[to].len())
                .min_by(|&a, &b| {
                    let ca = item_rect(to, a);
                    let cb = item_rect(to, b);
                    (ca.x + ca.w * 0.5 - x).abs().total_cmp(&(cb.x + cb.w * 0.5 - x).abs())
                })
                .unwrap_or(0)
        };
        match key {
            Key::Up if row > 0 => self.mode = Mode::Control(index_of(row - 1, nearest(row - 1))),
            Key::Down if row + 1 < ROWS.len() => self.mode = Mode::Control(index_of(row + 1, nearest(row + 1))),
            Key::Left | Key::Right if item == Item::Brightness => {
                let tv = self.tv_settings.as_mut()?;
                let step: i32 = if key == Key::Left { -5 } else { 5 };
                tv.brightness = (tv.brightness as i32 + step).clamp(0, 100) as u32;
                return Some(Action::TvControl(TvControl::Brightness(tv.brightness)));
            }
            Key::Left if column > 0 => self.mode = Mode::Control(focus - 1),
            Key::Right if column + 1 < ROWS[row].len() => self.mode = Mode::Control(focus + 1),
            Key::Back => self.mode = Mode::Browse,
            Key::Ok => return self.control_ok(item),
            _ => {}
        }
        None
    }

    fn control_ok(&mut self, item: Item) -> Option<Action> {
        let next = |list: &[String], current: &str| -> Option<String> {
            let at = list.iter().position(|m| m == current).map_or(0, |i| i + 1);
            list.get(at % list.len().max(1)).cloned()
        };
        match item {
            Item::Sound => {
                let tv = self.tv_settings.as_mut()?;
                tv.sound_mode = next(&tv.sound_modes, &tv.sound_mode)?;
                Some(Action::TvControl(TvControl::SoundMode(tv.sound_mode.clone())))
            }
            Item::Picture => {
                let tv = self.tv_settings.as_mut()?;
                tv.picture_mode = next(&tv.picture_modes, &tv.picture_mode)?;
                Some(Action::TvControl(TvControl::PictureMode(tv.picture_mode.clone())))
            }
            Item::Sleep => {
                let tv = self.tv_settings.as_mut()?;
                let list: Vec<String> = SLEEP.iter().map(|s| s.to_string()).collect();
                tv.sleep_timer = next(&list, &tv.sleep_timer)?;
                Some(Action::TvControl(TvControl::SleepTimer(tv.sleep_timer.clone())))
            }
            Item::ScreenOff => {
                self.mode = Mode::Browse;
                Some(Action::TvControl(TvControl::ScreenOff))
            }
            Item::Power => {
                self.mode = Mode::Browse;
                Some(Action::TvControl(TvControl::PowerOff))
            }
            Item::Theme => {
                self.mode = Mode::Browse;
                self.header = false;
                self.change_setting(SettingId::Theme)
            }
            Item::Wallpaper => self.change_setting(SettingId::Wallpaper),
            Item::Settings => {
                self.mode = Mode::Settings;
                self.settings_column = 0;
                self.ip = crate::net::local_ip();
                None
            }
            Item::Brightness => None,
        }
    }

    pub(super) fn control_pointer(&mut self, index: usize) -> bool {
        match &mut self.mode {
            Mode::Control(focus) => std::mem::replace(focus, index) != index,
            Mode::Inputs(focus) => std::mem::replace(focus, index) != index,
            _ => false,
        }
    }

    pub(super) fn draw_control(&self, r: &mut Renderer, text: &mut Text) {
        let Mode::Control(focus) = self.mode else { return };
        let open = self.overlay.value.clamp(0.0, 1.0);
        let s = self.strings;
        let white = Rgba::hex(0xFFFFFF);
        let dark = Rgba::hex(0x1C1C1E);
        r.rect(Rect::new(0.0, 0.0, LOGICAL_W, LOGICAL_H), 0.0, Rgba::hex(0).alpha(0.35 * open));
        let tv = self.tv_settings.as_ref();
        let dash = || "–".to_string();
        for index in 0..ROWS.iter().map(|r| r.len()).sum() {
            let (row, column, item) = item_at(index);
            let rect = item_rect(row, column).offset(0.0, -12.0 * (1.0 - open));
            self.hits.borrow_mut().push((rect, Target::ControlItem(index)));
            let focused = index == focus;
            let rect = if focused { rect.scale_about(1.04, 0.5, 0.5) } else { rect };
            let radius = rect.h * 0.5;
            if focused {
                r.shadow(rect.offset(0.0, 12.0), radius, 30.0, Rgba::hex(0).alpha(0.3 * open));
            }
            glass_surface(r, self.wallpaper, self.backdrop_t.value, rect, radius, open);
            if focused {
                r.rect(rect, radius, white.alpha(0.92 * open));
            }
            let color = if focused { dark } else { white };
            let (ch, title, value) = match item {
                Item::Sound => (icon::SOUND, s.sound, tv.map_or_else(dash, |t| crate::strings::sound_mode_name(&t.sound_mode))),
                Item::Picture => (icon::PICTURE, s.picture, tv.map_or_else(dash, |t| crate::strings::picture_mode_name(&t.picture_mode))),
                Item::Theme => (icon::THEME, s.theme, self.look.name().to_string()),
                Item::Sleep => (
                    icon::TIMER,
                    s.sleep_timer,
                    tv.map_or_else(dash, |t| if t.sleep_timer == "off" { s.off.to_string() } else { s.minutes.replace("{n}", &t.sleep_timer) }),
                ),
                Item::ScreenOff => (icon::SCREEN_OFF, s.screen_off, String::new()),
                Item::Power => (icon::POWER, s.power_off, String::new()),
                Item::Wallpaper => (icon::WALLPAPER, s.wallpaper, String::new()),
                Item::Settings => (icon::SETTINGS, s.settings, String::new()),
                Item::Brightness => (icon::BRIGHTNESS, s.brightness, tv.map_or_else(dash, |t| t.brightness.to_string())),
            };
            match ROWS[row].len() {
                // Circles: the icon alone (its name shows under the focused one).
                n if n > 2 => {
                    text.icon(r, ch, rect.x + rect.w * 0.5, rect.y + rect.h * 0.5, ICON, color.alpha(open));
                    if focused {
                        let w = text.measure(r, title, GLASS_SMALL);
                        text.draw_top(r, title, rect.x + rect.w * 0.5 - w * 0.5, rect.y + rect.h + 10.0, GLASS_SMALL, white.alpha(0.85 * open));
                    }
                }
                1 => {
                    // The brightness bar: filled to the value.
                    if let Some(t) = tv {
                        let fill = Rect::new(rect.x, rect.y, (rect.w * t.brightness as f32 / 100.0).max(rect.h), rect.h);
                        r.rect(fill, radius, if focused { dark.alpha(0.12 * open) } else { white.alpha(0.18 * open) });
                    }
                    text.icon(r, ch, rect.x + 50.0, rect.y + rect.h * 0.5, ICON, color.alpha(open));
                    text.draw(r, title, rect.x + 92.0, rect.y + rect.h * 0.5 + 11.0, GLASS_ROW, color.alpha(open));
                    let w = text.measure(r, &value, GLASS_ROW);
                    text.draw(r, &value, rect.x + rect.w - 40.0 - w, rect.y + rect.h * 0.5 + 11.0, GLASS_ROW, color.alpha(open));
                }
                _ => {
                    text.icon(r, ch, rect.x + 50.0, rect.y + rect.h * 0.5, ICON, color.alpha(open));
                    let width = rect.w - 104.0;
                    let title = clip(text, r, title, GLASS_ROW, width);
                    let value = clip(text, r, &value, GLASS_SMALL, width);
                    text.draw(r, &title, rect.x + 88.0, rect.y + 44.0, GLASS_ROW, color.alpha(open));
                    text.draw(r, &value, rect.x + 88.0, rect.y + 76.0, GLASS_SMALL, color.alpha(0.65 * open));
                }
            }
        }
    }
}
