//! The Homebrew page: apps from the Homebrew Channel repository (repo.webosbrew.org), as a
//! grid with tabs (all, installed, updates) and a detail panel to install, update, open or
//! uninstall. Installs are verified against the repository's SHA-256.
use super::*;
use crate::model::{HomebrewApp, HOMEBREW_CHANNEL};

/// The tile that opens this page; it sits among the apps (movable, hideable).
pub(super) const HOMEBREW_TILE: &str = "vision.homebrew";

const COLS: usize = 4;
const CARD: (f32, f32) = (400.0, 150.0);
const GAP: f32 = 32.0;
const GRID_X: f32 = 112.0;
const GRID_Y: f32 = 270.0;
const PITCH: f32 = CARD.1 + 30.0;
const ICON: f32 = 96.0;
const BRW_ATLAS: (i32, i32) = (2048, 1024);
const BRW_CELL: i32 = 128;
const WHITE: Rgba = Rgba::hex(0xFFFFFF);

/// Which part of the page has focus.
pub(super) struct Brew {
    tab: usize,
    /// 0: the tabs, 1: the grid.
    row: usize,
    focus: usize,
    /// The app shown in the detail panel.
    detail: Option<String>,
    button: usize,
    /// Uninstall was pressed once; the next press confirms.
    confirm: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum BrewButton {
    Install,
    Update,
    Retry,
    Open,
    Uninstall,
}

impl Home {
    pub(super) fn open_homebrew(&mut self) -> Option<Action> {
        self.mode = Mode::Homebrew(Brew { tab: 0, row: 1, focus: 0, detail: None, button: 0, confirm: false });
        self.brew_scroll = Spring::new(0.0, 16.0);
        // Loaded once per start (and again after a failure).
        match &self.brew {
            Some(Ok(_)) => None,
            _ => {
                self.brew = None;
                Some(Action::Homebrew)
            }
        }
    }

    pub fn set_homebrew(&mut self, apps: Result<Vec<HomebrewApp>, String>) {
        self.brew = Some(apps);
        self.refocus(false);
    }

    pub fn add_brew_icon(&mut self, icon: crate::model::Icon, renderer: &Renderer) {
        let per_row = BRW_ATLAS.0 / BRW_CELL;
        if self.brew_icons.contains_key(&icon.app_id) || self.brew_next >= per_row * (BRW_ATLAS.1 / BRW_CELL) {
            return;
        }
        let atlas = *self.brew_atlas.get_or_insert_with(|| renderer.create_texture(BRW_ATLAS.0, BRW_ATLAS.1, gl::RGBA));
        let (x, y) = ((self.brew_next % per_row) * BRW_CELL, (self.brew_next / per_row) * BRW_CELL);
        self.brew_next += 1;
        renderer.upload(atlas, x, y, icon.w as i32, icon.h as i32, gl::RGBA, &icon.pixels);
        let (aw, ah) = (BRW_ATLAS.0 as f32, BRW_ATLAS.1 as f32);
        let uv = [x as f32 / aw, y as f32 / ah, (x + icon.w as i32) as f32 / aw, (y + icon.h as i32) as f32 / ah];
        self.brew_icons.insert(icon.app_id, uv);
    }

    /// Progress of an install or uninstall started from this page.
    pub fn set_brew_install(&mut self, app: String, state: InstallState) {
        let s = self.strings;
        let title = self.brew_app(&app).map_or(app.clone(), |a| a.title.clone());
        match state {
            InstallState::Done => {
                let installed = self.versions.contains_key(&app);
                self.brew_progress.remove(&app);
                let message = if installed { s.brew_removed } else { s.brew_done };
                self.notice = Some((format!("{message}: {title}"), Instant::now()));
                if let Mode::Homebrew(brew) = &mut self.mode {
                    brew.confirm = false;
                    brew.button = 0;
                }
                // The new app list brings the new state.
                self.pending.push(Action::Refresh);
            }
            state => {
                self.brew_progress.insert(app, state);
            }
        }
        self.refocus(false);
    }

    pub fn set_versions(&mut self, versions: Vec<(String, String)>) {
        self.versions = versions.into_iter().collect();
    }

    fn brew_app(&self, id: &str) -> Option<&HomebrewApp> {
        self.brew.as_ref()?.as_ref().ok()?.iter().find(|a| a.id == id)
    }

    /// The apps a tab lists, as indices into the repository list.
    fn brew_list(&self, tab: usize) -> Vec<usize> {
        let Some(Ok(apps)) = &self.brew else { return Vec::new() };
        (0..apps.len())
            .filter(|&i| {
                let installed = self.versions.get(&apps[i].id);
                match tab {
                    1 => installed.is_some(),
                    2 => installed.is_some_and(|v| version_newer(&apps[i].package.version, v)),
                    _ => true,
                }
            })
            .collect()
    }

    fn brew_buttons(&self, app: &HomebrewApp) -> Vec<BrewButton> {
        let mut buttons = Vec::new();
        let busy = matches!(self.brew_progress.get(&app.id), Some(InstallState::Downloading(_) | InstallState::Installing | InstallState::Removing));
        if busy {
            return buttons;
        }
        let failed = matches!(self.brew_progress.get(&app.id), Some(InstallState::Failed(_)));
        match self.versions.get(&app.id) {
            None => buttons.push(if failed { BrewButton::Retry } else { BrewButton::Install }),
            Some(v) if version_newer(&app.package.version, v) => buttons.push(if failed { BrewButton::Retry } else { BrewButton::Update }),
            Some(_) => buttons.push(BrewButton::Open),
        }
        // Vision and Homebrew Channel stay: without them nothing here works.
        if self.versions.contains_key(&app.id) && app.id != HOMEBREW_CHANNEL && app.id != crate::platform::APP_ID {
            buttons.push(BrewButton::Uninstall);
        }
        buttons
    }

    pub(super) fn homebrew_key(&mut self, key: Key) -> Option<Action> {
        let Mode::Homebrew(brew) = &self.mode else { return None };
        let (tab, row, focus, detail, button, confirm) = (brew.tab, brew.row, brew.focus, brew.detail.clone(), brew.button, brew.confirm);
        let list = self.brew_list(tab);

        if let Some(id) = detail {
            let Some(app) = self.brew_app(&id).cloned() else { return None };
            let buttons = self.brew_buttons(&app);
            let Mode::Homebrew(brew) = &mut self.mode else { return None };
            match key {
                Key::Left if button > 0 => {
                    brew.button -= 1;
                    brew.confirm = false;
                }
                Key::Right if button + 1 < buttons.len() => {
                    brew.button += 1;
                    brew.confirm = false;
                }
                Key::Back => {
                    brew.detail = None;
                    brew.confirm = false;
                }
                Key::Ok => match buttons.get(button.min(buttons.len().saturating_sub(1))) {
                    Some(BrewButton::Install | BrewButton::Update | BrewButton::Retry) => {
                        self.brew_progress.insert(app.id.clone(), InstallState::Downloading(0.0));
                        return Some(Action::HomebrewInstall(app.package.clone()));
                    }
                    Some(BrewButton::Open) => {
                        self.mode = Mode::Browse;
                        return Some(Action::Launch { app_id: app.id.clone(), params: None });
                    }
                    Some(BrewButton::Uninstall) if confirm => {
                        brew.confirm = false;
                        self.brew_progress.insert(app.id.clone(), InstallState::Removing);
                        return Some(Action::HomebrewRemove(app.id.clone()));
                    }
                    Some(BrewButton::Uninstall) => brew.confirm = true,
                    None => {}
                },
                _ => {}
            }
            return None;
        }

        let Mode::Homebrew(brew) = &mut self.mode else { return None };
        match (key, row) {
            (Key::Back, _) => {
                self.mode = Mode::Browse;
                return None;
            }
            // A failed load: OK tries again.
            (Key::Ok, _) if matches!(self.brew, Some(Err(_))) => {
                self.brew = None;
                return Some(Action::Homebrew);
            }
            (Key::Left, 0) if tab > 0 => {
                brew.tab -= 1;
                brew.focus = 0;
            }
            (Key::Right, 0) if tab < 2 => {
                brew.tab += 1;
                brew.focus = 0;
            }
            (Key::Down, 0) if !list.is_empty() => brew.row = 1,
            (Key::Left, 1) if focus % COLS > 0 => brew.focus -= 1,
            (Key::Right, 1) if focus % COLS + 1 < COLS && focus + 1 < list.len() => brew.focus += 1,
            (Key::Up, 1) if focus >= COLS => brew.focus -= COLS,
            (Key::Up, 1) => brew.row = 0,
            (Key::Down, 1) if focus + COLS < list.len() => brew.focus += COLS,
            (Key::Down, 1) if focus / COLS < list.len().saturating_sub(1) / COLS => brew.focus = list.len() - 1,
            (Key::Ok, 1) => {
                if let Some(&i) = list.get(focus) {
                    let id = self.brew.as_ref().and_then(|b| b.as_ref().ok()).map(|apps| apps[i].id.clone());
                    if let Mode::Homebrew(brew) = &mut self.mode {
                        brew.detail = id;
                        brew.button = 0;
                        brew.confirm = false;
                    }
                }
            }
            _ => {}
        }
        None
    }

    /// Keeps the focused card's row on screen.
    pub(super) fn brew_refocus(&mut self) {
        let Mode::Homebrew(brew) = &self.mode else { return };
        let row = (brew.focus / COLS) as f32;
        let y = GRID_Y + row * PITCH;
        let bottom = LOGICAL_H - 80.0;
        let mut target = self.brew_scroll.target;
        if y - target < GRID_Y {
            target = y - GRID_Y;
        } else if y + CARD.1 - target > bottom {
            target = y + CARD.1 - bottom;
        }
        self.brew_scroll.target = target.max(0.0);
    }

    pub(super) fn brew_pointer(&mut self, target: Target) -> bool {
        let Mode::Homebrew(brew) = &mut self.mode else { return false };
        let before = (brew.tab, brew.row, brew.focus, brew.button);
        match target {
            Target::BrewTab(i) if brew.detail.is_none() => {
                if brew.tab != i {
                    brew.focus = 0;
                }
                brew.tab = i;
                brew.row = 0;
            }
            Target::BrewCard(i) if brew.detail.is_none() => {
                brew.row = 1;
                brew.focus = i;
            }
            Target::BrewButton(i) if brew.detail.is_some() => brew.button = i,
            _ => {}
        }
        before != (brew.tab, brew.row, brew.focus, brew.button)
    }

    pub(super) fn draw_homebrew(&self, r: &mut Renderer, text: &mut Text) {
        let open = self.overlay.value.clamp(0.0, 1.0);
        let Mode::Homebrew(brew) = &self.mode else { return };
        if open <= 0.01 {
            return;
        }
        let s = self.strings;
        let a = |v: f32| WHITE.alpha(v * open);
        r.rect(Rect::new(0.0, 0.0, LOGICAL_W, LOGICAL_H), 0.0, Rgba::hex(0x0E0F13).alpha(open));
        text.draw_top(r, "Homebrew", GRID_X, 80.0, SETUP_TITLE, a(1.0));
        text.draw_top(r, "repo.webosbrew.org", GRID_X + 6.0, 150.0, SETUP_SMALL, a(0.45));

        // Tabs, with the number of updates.
        let updates = self.brew_list(2).len();
        let mut x = LOGICAL_W - GRID_X;
        for i in (0..3).rev() {
            let label = if i == 2 && updates > 0 { format!("{} ({updates})", s.brew_tabs[2]) } else { s.brew_tabs[i].to_string() };
            let w = text.measure(r, &label, MENU_ITEM) + 56.0;
            x -= w;
            let pill = Rect::new(x, 96.0, w, 60.0);
            self.hits.borrow_mut().push((pill, Target::BrewTab(i)));
            let current = i == brew.tab;
            let focused = current && brew.row == 0 && brew.detail.is_none();
            if focused {
                r.rect(pill, 30.0, a(1.0));
            } else if current {
                r.rect(pill, 30.0, a(0.14));
            }
            text.draw(r, &label, x + 28.0, 96.0 + 40.0, MENU_ITEM, if focused { Rgba::hex(0x111114).alpha(open) } else { a(0.9) });
            x -= 16.0;
        }

        let apps = match &self.brew {
            None => {
                text.draw_top(r, s.brew_loading, GRID_X, GRID_Y, SETUP_BODY, a(0.7));
                return;
            }
            Some(Err(_)) => {
                text.draw_top(r, s.brew_failed, GRID_X, GRID_Y, SETUP_BODY, a(0.8));
                text.draw_top(r, s.press_ok_to_retry, GRID_X, GRID_Y + 50.0, SETUP_SMALL, a(0.5));
                return;
            }
            Some(Ok(apps)) => apps,
        };
        let list = self.brew_list(brew.tab);
        if list.is_empty() {
            text.draw_top(r, s.brew_empty, GRID_X, GRID_Y, SETUP_BODY, a(0.6));
        }
        let scroll = self.brew_scroll.value;
        // Passes: shapes, icons (one texture), text.
        for pass in 0..3 {
            for (slot, &i) in list.iter().enumerate() {
                let app = &apps[i];
                let base = Rect::new(GRID_X + (slot % COLS) as f32 * (CARD.0 + GAP), GRID_Y + (slot / COLS) as f32 * PITCH - scroll, CARD.0, CARD.1);
                if base.y > LOGICAL_H || base.y + base.h < 200.0 {
                    continue;
                }
                let focused = brew.row == 1 && slot == brew.focus && brew.detail.is_none();
                let card = if focused { base.scale_about(1.04, 0.5, 0.5) } else { base };
                let icon = Rect::new(card.x + 24.0, card.y + (card.h - ICON) * 0.5, ICON, ICON);
                match pass {
                    0 => {
                        self.hits.borrow_mut().push((card, Target::BrewCard(slot)));
                        if focused {
                            r.shadow(card.offset(0.0, 14.0), 20.0, 36.0, Rgba::hex(0).alpha(0.5 * open));
                        }
                        r.rect(card, 20.0, Rgba::hex(0x1C1E25).alpha(open));
                        if focused {
                            r.stroke(card.inflate(1.5), 21.5, 3.0, a(1.0));
                        }
                        if !self.brew_icons.contains_key(&app.id) {
                            r.rect(icon, 22.0, Rgba::hex(0x2C2F38).alpha(open));
                        }
                    }
                    1 => {
                        if let (Some(atlas), Some(uv)) = (self.brew_atlas, self.brew_icons.get(&app.id)) {
                            r.image(atlas, icon, *uv, 22.0, a(1.0));
                        }
                    }
                    _ => {
                        let tx = icon.x + ICON + 22.0;
                        let width = card.x + card.w - tx - 20.0;
                        let badge = self.brew_badge(app).map(|(label, color)| (label, color, text.measure(r, label, SETUP_SMALL) + 20.0));
                        // The title stops short of the badge.
                        let title_width = badge.map_or(width, |(_, _, w)| width - w + 2.0);
                        let title = clip(text, r, &app.title, ENTITY_NAME, title_width);
                        text.draw_top(r, &title, tx, card.y + 26.0, ENTITY_NAME, a(1.0));
                        for (l, line) in wrap(text, r, &app.summary, INPUT_SUB, width).iter().take(2).enumerate() {
                            text.draw_top(r, line, tx, card.y + 66.0 + l as f32 * 26.0, INPUT_SUB, a(0.6));
                        }
                        if let Some((label, color, w)) = badge {
                            let pill = Rect::new(card.x + card.w - w - 14.0, card.y + 12.0, w, 30.0);
                            r.rect(pill, 15.0, color.alpha(open));
                            text.draw(r, label, pill.x + 10.0, pill.y + 22.0, SETUP_SMALL, Rgba::hex(0x101114).alpha(open));
                        }
                    }
                }
            }
        }
        if let Some(id) = &brew.detail {
            if let Some(app) = self.brew_app(id) {
                self.draw_brew_detail(r, text, app, brew, open);
            }
        }
    }

    /// Installed, update or root — the one badge a card shows.
    fn brew_badge(&self, app: &HomebrewApp) -> Option<(&'static str, Rgba)> {
        let s = self.strings;
        match self.versions.get(&app.id) {
            Some(v) if version_newer(&app.package.version, v) => Some((s.brew_update_badge, Rgba::hex(0xF2C14E))),
            Some(_) => Some((s.brew_installed_badge, Rgba::hex(0x4ADE80))),
            None if app.root => Some((s.brew_root_badge, Rgba::hex(0xFF8A80))),
            None => None,
        }
    }

    fn draw_brew_detail(&self, r: &mut Renderer, text: &mut Text, app: &HomebrewApp, brew: &Brew, open: f32) {
        let s = self.strings;
        let a = |v: f32| WHITE.alpha(v * open);
        r.rect(Rect::new(0.0, 0.0, LOGICAL_W, LOGICAL_H), 0.0, Rgba::hex(0).alpha(0.6 * open));
        let panel = Rect::new(260.0, 150.0, 1400.0, 780.0);
        self.panel(r, panel, open);
        let icon = Rect::new(panel.x + 64.0, panel.y + 64.0, 160.0, 160.0);
        match (self.brew_atlas, self.brew_icons.get(&app.id)) {
            (Some(atlas), Some(uv)) => r.image(atlas, icon, *uv, 34.0, a(1.0)),
            _ => r.rect(icon, 34.0, Rgba::hex(0x2C2F38).alpha(open)),
        }
        let x = icon.x + icon.w + 48.0;
        let width = panel.x + panel.w - 64.0 - x;
        let title = clip(text, r, &app.title, SETUP_TITLE, width);
        text.draw_top(r, &title, x, icon.y + 6.0, SETUP_TITLE, a(1.0));
        let mut facts = vec![format!("{} {}", s.version, app.package.version)];
        if let Some(size) = app.package.size {
            facts.push(format!("{:.1} MB", size as f32 / 1_048_576.0));
        }
        if let Some(installed) = self.versions.get(&app.id) {
            facts.push(format!("{} {installed}", s.store_installed));
        }
        text.draw_top(r, &facts.join(" · "), x, icon.y + 84.0, SETUP_SMALL, a(0.6));
        if app.root {
            let note = clip(text, r, s.brew_root_note, SETUP_SMALL, width);
            text.draw_top(r, &note, x, icon.y + 122.0, SETUP_SMALL, Rgba::hex(0xFF8A80).alpha(open));
        }

        let description = if app.description.is_empty() { &app.summary } else { &app.description };
        let mut y = icon.y + icon.h + 48.0;
        for line in wrap(text, r, description, SETUP_BODY, panel.w - 128.0).iter().take(6) {
            text.draw_top(r, line, panel.x + 64.0, y, SETUP_BODY, a(0.85));
            y += 42.0;
        }
        if !app.source.is_empty() {
            let source = clip(text, r, &app.source, SETUP_SMALL, panel.w - 128.0);
            text.draw_top(r, &source, panel.x + 64.0, y + 8.0, SETUP_SMALL, a(0.45));
        }

        // Buttons, or the progress of what is running.
        let by = panel.y + panel.h - 64.0 - 64.0;
        let mut bx = panel.x + 64.0;
        match self.brew_progress.get(&app.id) {
            Some(InstallState::Downloading(p)) => self.brew_bar(r, text, bx, by, *p, &format!("{} {:.0} %", s.store_downloading, p * 100.0), open),
            Some(InstallState::Installing) => self.brew_bar(r, text, bx, by, 1.0, s.store_installing, open),
            Some(InstallState::Removing) => self.brew_bar(r, text, bx, by, 1.0, s.brew_removing, open),
            Some(InstallState::Failed(failure)) => {
                let message = match failure {
                    crate::model::InstallFailure::Network => s.fail_network,
                    crate::model::InstallFailure::Damaged => s.fail_damaged,
                    crate::model::InstallFailure::NeedsRoot => s.fail_root,
                    crate::model::InstallFailure::Refused => s.fail_refused,
                };
                let line = clip(text, r, message, SETUP_SMALL, panel.w - 128.0);
                text.draw_top(r, &line, panel.x + 64.0, by - 40.0, SETUP_SMALL, Rgba::hex(0xFF8A80).alpha(open));
            }
            _ => {}
        }
        let buttons = self.brew_buttons(app);
        let focus = brew.button.min(buttons.len().saturating_sub(1));
        for (i, button) in buttons.iter().enumerate() {
            let label = match button {
                BrewButton::Install => s.store_install,
                BrewButton::Update => s.store_update,
                BrewButton::Retry => s.store_retry,
                BrewButton::Open => s.brew_open,
                BrewButton::Uninstall if brew.confirm => s.brew_confirm,
                BrewButton::Uninstall => s.brew_uninstall,
            };
            let w = text.measure(r, label, MENU_ITEM) + 72.0;
            let rect = Rect::new(bx, by, w, 64.0);
            self.hits.borrow_mut().push((rect, Target::BrewButton(i)));
            let focused = i == focus;
            let danger = *button == BrewButton::Uninstall && brew.confirm;
            if focused {
                r.rect(rect, 32.0, if danger { Rgba::hex(0xFF6B6B) } else { WHITE }.alpha(open));
            } else {
                r.stroke(rect, 32.0, 1.5, a(0.35));
            }
            text.draw(r, label, bx + 36.0, by + 42.0, MENU_ITEM, if focused { Rgba::hex(0x111114).alpha(open) } else { a(1.0) });
            bx += w + 20.0;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn brew_bar(&self, r: &mut Renderer, text: &mut Text, x: f32, y: f32, progress: f32, label: &str, open: f32) {
        let track = Rect::new(x, y, 460.0, 64.0);
        r.rect(track, 32.0, WHITE.alpha(0.1 * open));
        r.rect(Rect::new(track.x, track.y, (track.w * progress).max(64.0), track.h), 32.0, Rgba::hex(0x4ADE80).alpha(0.85 * open));
        text.draw(r, label, track.x + 32.0, track.y + 42.0, MENU_ITEM, WHITE.alpha(open));
    }
}
