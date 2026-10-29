//! Remote navigation for the Glass look: the round buttons at the top (inputs, Control Center),
//! the Top Shelf of the focused app in the dock, the app grid and the rows under it.
use super::glass::COLS;
use super::*;

impl Home {
    pub(super) fn glass_browse_key(&mut self, key: Key, home_mode: bool) -> Option<Action> {
        if self.header {
            self.buttons_key(key)
        } else if self.active == FAVORITES {
            self.grid_key(key, home_mode)
        } else if self.in_shelf() {
            self.shelf_key(key)
        } else {
            self.rows_key(key)
        }
    }

    fn focus_grid(&mut self, index: usize) {
        self.active = FAVORITES;
        self.header = false;
        self.rails[FAVORITES].focus = index.min(self.rails[FAVORITES].tiles.len().saturating_sub(1));
    }

    /// The round buttons at the top right: OK opens the inputs or the Control Center.
    fn buttons_key(&mut self, key: Key) -> Option<Action> {
        match key {
            Key::Ok => return self.open_header_item(),
            Key::Left => self.header_item = 0,
            Key::Right => self.header_item = 1,
            Key::Down | Key::Back => {
                self.header = false;
                self.active = FAVORITES;
            }
            _ => {}
        }
        None
    }

    fn grid_key(&mut self, key: Key, home_mode: bool) -> Option<Action> {
        let index = self.grid_index();
        let total = self.grid_total();
        let column = index % COLS;
        match key {
            Key::Left if column > 0 => self.focus_grid(index - 1),
            Key::Right if column + 1 < COLS && index + 1 < total => self.focus_grid(index + 1),
            Key::Up if index >= COLS => self.focus_grid(index - COLS),
            // From the dock into the app's Top Shelf content if it has any, else to the buttons.
            Key::Up => match self.shelf_rails().iter().find(|&&r| self.has(r)) {
                Some(&rail) => {
                    self.active = rail;
                    self.rails[rail].focus = 0;
                }
                None => {
                    self.header = true;
                    self.header_item = 1;
                }
            },
            Key::Down if index + COLS < total => self.focus_grid(index + COLS),
            // A shorter last row: its last tile.
            Key::Down if total > 0 && index / COLS < (total - 1) / COLS => self.focus_grid(total - 1),
            Key::Down => {
                let first = self.glass_rows().next();
                if let Some(rail) = first {
                    let (x, _) = Self::grid_xy(column as f32, 0.0);
                    self.focus_row(rail, x + glass::TILE.0 * 0.5);
                }
            }
            Key::Ok => return self.activate(),
            Key::LongOk if !self.rails[FAVORITES].tiles.is_empty() => {
                self.mode = Mode::Menu(Menu { items: vec![MenuItem::Move, MenuItem::Hide, MenuItem::Cancel], focus: 0 });
            }
            Key::Back if index != 0 => self.focus_grid(0),
            Key::Back if !home_mode => return Some(Action::Exit),
            _ => {}
        }
        None
    }

    /// The shelf's sections form one strip: Left/Right cross from one section to the next.
    fn shelf_key(&mut self, key: Key) -> Option<Action> {
        let strip: Vec<usize> = self.shelf_rails().iter().copied().filter(|&r| self.has(r)).collect();
        let at = strip.iter().position(|&r| r == self.active).unwrap_or(0);
        let focus = self.rails[self.active].focus;
        match key {
            Key::Right if focus + 1 < self.rails[self.active].tiles.len() => self.rails[self.active].focus += 1,
            Key::Right if at + 1 < strip.len() => {
                self.active = strip[at + 1];
                self.rails[self.active].focus = 0;
            }
            Key::Left if focus > 0 => self.rails[self.active].focus -= 1,
            Key::Left if at > 0 => {
                self.active = strip[at - 1];
                self.rails[self.active].focus = self.rails[self.active].tiles.len().saturating_sub(1);
            }
            // Back to the app (its focus never left it); above the shelf are the buttons.
            Key::Down | Key::Back => self.active = FAVORITES,
            Key::Up => {
                self.active = FAVORITES;
                self.header = true;
                self.header_item = 1;
            }
            Key::Ok => return self.activate(),
            _ => {}
        }
        None
    }

    fn rows_key(&mut self, key: Key) -> Option<Action> {
        let rows: Vec<usize> = self.glass_rows().collect();
        let rail = self.active;
        let at = rows.iter().position(|&r| r == rail).unwrap_or(0);
        let focus = self.rails[rail].focus;
        let count = self.rails[rail].tiles.len();
        match key {
            Key::Left if focus > 0 => self.rails[rail].focus -= 1,
            Key::Right if focus + 1 < count => self.rails[rail].focus += 1,
            Key::Up if at > 0 => self.focus_row(rows[at - 1], self.row_center(rail, focus)),
            // Up from the first row: the grid's last row, at the nearest column.
            Key::Up => {
                let x = self.row_center(rail, focus);
                let column = ((x - glass::X0) / (glass::TILE.0 + glass::GAP)).floor().clamp(0.0, (COLS - 1) as f32) as usize;
                let last_row = self.grid_total().saturating_sub(1) / COLS;
                self.focus_grid(last_row * COLS + column);
            }
            Key::Down if at + 1 < rows.len() => self.focus_row(rows[at + 1], self.row_center(rail, focus)),
            Key::Ok => return self.activate(),
            Key::LongOk if rail == HIDDEN && count > 0 => {
                self.mode = Mode::Menu(Menu { items: vec![MenuItem::Show, MenuItem::Cancel], focus: 0 });
            }
            Key::LongOk if self.focused().is_some_and(|t| t.kind == Kind::Promo) => {
                self.mode = Mode::Menu(Menu { items: vec![MenuItem::HidePromo, MenuItem::Cancel], focus: 0 });
            }
            Key::Back => self.focus_grid(0),
            _ => {}
        }
        None
    }

    /// Focuses the tile of `rail` nearest to `x` (a tile centre in the row above or below).
    fn focus_row(&mut self, rail: usize, x: f32) {
        let count = self.rails[rail].tiles.len();
        let nearest = (0..count)
            .min_by(|&a, &b| (self.row_center(rail, a) - x).abs().total_cmp(&(self.row_center(rail, b) - x).abs()))
            .unwrap_or(0);
        self.header = false;
        self.active = rail;
        self.rails[rail].focus = nearest;
    }

    /// Moving an app in the grid: Left/Right by one place, Up/Down by a row (taking the app
    /// out and putting it back in, so the others close up like on tvOS).
    pub(super) fn glass_move_key(&mut self, key: Key) -> Option<Action> {
        let rail = &mut self.rails[FAVORITES];
        let focus = rail.focus;
        let count = rail.tiles.len();
        let action = match key {
            Key::Up if focus >= COLS => {
                let tile = rail.tiles.remove(focus);
                rail.tiles.insert(focus - COLS, tile);
                rail.focus = focus - COLS;
                None
            }
            Key::Down if focus + COLS < count => {
                let tile = rail.tiles.remove(focus);
                rail.tiles.insert(focus + COLS, tile);
                rail.focus = focus + COLS;
                None
            }
            Key::Up | Key::Down => None,
            _ => self.move_key(key),
        };
        self.glass_reslot();
        action
    }
}
