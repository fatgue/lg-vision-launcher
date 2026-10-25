//! Overlays drawn over the home screen: menu, setup panels, offer, store page, settings,
//! entity picker, digit entry and notices.
use super::*;

impl Home {
    pub(super) fn draw_menu(&self, r: &mut Renderer, text: &mut Text) {
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

    pub(super) fn draw_setup(&self, r: &mut Renderer, text: &mut Text) {
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

    pub(super) fn draw_offer(&self, r: &mut Renderer, text: &mut Text) {
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
    pub(super) fn draw_store(&self, r: &mut Renderer, text: &mut Text) {
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

    pub(super) fn draw_settings(&self, r: &mut Renderer, text: &mut Text) {
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

    pub(super) fn draw_picker(&self, r: &mut Renderer, text: &mut Text) {
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
    pub(super) fn draw_digits(&self, r: &mut Renderer, text: &mut Text) {
        let Some((entry, _)) = &self.digits else { return };
        let label: String = entry.chars().map(|c| format!("{c} ")).collect::<String>() + "_";
        let w = text.measure(r, &label, DIGITS) + 64.0;
        let rect = Rect::new(LOGICAL_W - SAFE_X - w, LOGICAL_H - 54.0 - 96.0, w, 96.0);
        r.rect(rect, 48.0, Rgba::hex(0x14161C).alpha(0.94));
        r.stroke(rect, 48.0, 1.0, TEXT.alpha(0.16));
        text.draw(r, &label, rect.x + 32.0, rect.y + 66.0, DIGITS, TEXT);
    }

    pub(super) fn draw_notice(&self, r: &mut Renderer, text: &mut Text) {
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
