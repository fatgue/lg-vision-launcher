//! Shared drawing primitives: panels, buttons, QR codes and text helpers.
use super::*;

/// Top-left texel of a store picture's cell (two per row).
pub(super) fn picture_cell(index: usize) -> (i32, i32) {
    ((index % 2) as i32 * PICTURE_CELL.0, (index / 2) as i32 * PICTURE_CELL.1)
}

pub(super) fn draw_panel(r: &mut Renderer, panel: Rect, open: f32) {
    r.shadow(panel.offset(0.0, 24.0), 22.0, 60.0, Rgba::hex(0).alpha(0.5 * open));
    r.rect(panel, 22.0, Rgba::hex(0x14161C).alpha(0.96 * open));
    r.stroke(panel, 22.0, 1.0, TEXT.alpha(0.14 * open));
}

pub(super) fn draw_button(r: &mut Renderer, text: &mut Text, label: &str, x: f32, y: f32, open: f32) -> Rect {
    let w = text.measure(r, label, MENU_ITEM) + 72.0;
    let rect = Rect::new(x, y, w, 64.0);
    r.rect(rect, 32.0, TEXT.alpha(open));
    text.draw(r, label, x + 36.0, y + 42.0, MENU_ITEM, BG.alpha(open));
    rect
}

/// QR modules as merged horizontal runs on a white quiet-zone card.
pub(super) fn draw_qr(r: &mut Renderer, qr: &qrcodegen::QrCode, area: Rect, open: f32) {
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
pub(super) fn wrap(text: &mut Text, r: &Renderer, line: &str, style: Style, width: f32) -> Vec<String> {
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

/// Shortens `label` with an ellipsis to fit `width`.
pub(super) fn clip(text: &mut Text, r: &Renderer, label: &str, style: Style, width: f32) -> String {
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

pub(super) fn fit(text: &mut Text, r: &Renderer, label: &str, style: Style, width: f32) -> Style {
    let mut style = style;
    while style.size > 20.0 && text.measure(r, label, style) > width {
        style.size -= 4.0;
    }
    style
}
