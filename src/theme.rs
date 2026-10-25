//! Design tokens for direction A, "Cinematic Ambient".
use crate::color::Rgba;
use crate::text::{Style, Weight};

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

pub fn hero(size: f32) -> Style {
    Style::new(Weight::SemiBold, size).tracking(-0.02)
}
