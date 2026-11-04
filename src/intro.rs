//! The intro on a cold start: "VISION" in steel, extruded into depth, drifting towards the
//! camera while a light sweeps across it with an anamorphic flare, then fading into Home.
//! Drawn live with the normal renderer (no video, no extra files); any key skips it. It only
//! plays when the process starts (power on, after an update), never on the Home key.
use crate::color::Rgba;
use crate::render::{Rect, Renderer, LOGICAL_H, LOGICAL_W};
use crate::text::Text;
use crate::theme::INTRO;

const WORD: &str = "VISION";
/// Seconds: the whole intro, and the fade into Home at its end.
const LENGTH: f32 = 2.4;
const FADE: f32 = 0.5;
/// Layers behind the face that give the letters their depth.
const DEPTH: usize = 16;
const PARTICLES: usize = 64;

pub struct Intro {
    t: f32,
}

fn smooth(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A repeatable pseudo-random number in 0..1.
fn hash(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

impl Intro {
    pub fn new() -> Self {
        Self { t: 0.0 }
    }

    /// Advances the clock; false once it is over.
    pub fn step(&mut self, dt: f32) -> bool {
        self.t += dt;
        self.t < LENGTH
    }

    /// Any key: straight to the fade.
    pub fn skip(&mut self) {
        self.t = self.t.max(LENGTH - FADE);
    }

    pub fn draw(&self, r: &mut Renderer, text: &mut Text) {
        let t = self.t;
        let shown = 1.0 - smooth(LENGTH - FADE, LENGTH, t);
        if shown <= 0.0 {
            return;
        }
        let screen = Rect::new(0.0, 0.0, LOGICAL_W, LOGICAL_H);
        let (cx, cy) = (LOGICAL_W * 0.5, LOGICAL_H * 0.5);
        r.rect(screen, 0.0, Rgba::hex(0x030406).alpha(shown));

        // Light from deep in the room, growing as the word arrives.
        let glow = smooth(0.0, 1.4, t);
        r.shadow(Rect::new(cx - 420.0, cy - 120.0, 840.0, 240.0), 120.0, 360.0, Rgba::hex(0x1C3A6E).alpha(0.55 * glow * shown));

        // Dust drifting up through the light.
        for i in 0..PARTICLES as u32 {
            let x = hash(i * 3 + 1) * LOGICAL_W;
            let speed = 18.0 + 40.0 * hash(i * 3 + 2);
            let y = (hash(i * 3 + 3) * LOGICAL_H - t * speed).rem_euclid(LOGICAL_H);
            let near = 1.0 - ((x - cx).abs() / cx).min(1.0) * 0.6;
            let size = 1.5 + 2.5 * hash(i * 7 + 5);
            let alpha = 0.35 * near * smooth(0.0, 0.8, t) * shown;
            r.rect(Rect::new(x, y, size, size), size * 0.5, Rgba::hex(0xBFD4FF).alpha(alpha));
        }

        // The camera moves slowly towards the word and, at the end, flies through it: the word
        // grows and is gone a little before the room fades into Home.
        let out = smooth(LENGTH - FADE - 0.15, LENGTH - 0.1, t);
        let scale = (0.80 + 0.16 * smooth(0.0, LENGTH - FADE, t)) * (1.0 + 0.6 * out * out);
        let word = 1.0 - smooth(LENGTH - FADE - 0.15, LENGTH - 0.2, t);
        let baseline = cy + 70.0 * scale;
        let count = WORD.chars().count();
        // Letters arrive one after the other.
        let arrive = |i: usize| smooth(0.10 + 0.07 * i as f32, 0.65 + 0.07 * i as f32, t) * word;

        // Depth: the same word repeated behind the face, each layer a little smaller and lower,
        // darkening towards the back.
        for layer in (1..=DEPTH).rev() {
            let k = layer as f32;
            let shade = Rgba::hex(0x2A313D).lerp(Rgba::hex(0x07090D), k / DEPTH as f32);
            text.draw_scaled(r, WORD, cx, baseline + 1.6 * k * scale, INTRO, scale * (1.0 - 0.0035 * k), |i, _| {
                shade.alpha(arrive(i) * shown)
            });
        }

        // The face: brushed steel, with a band of light sweeping from left to right.
        let sweep = -0.25 + 1.5 * smooth(0.55, 1.75, t);
        text.draw_scaled(r, WORD, cx, baseline, INTRO, scale, |i, x| {
            let light = (-((x - sweep) / 0.13).powi(2)).exp();
            let steel = Rgba::hex(0x9AA4B4).lerp(Rgba::hex(0xE8EEF7), 0.35 + 0.65 * (i as f32 / count as f32 - 0.5).abs());
            steel.lerp(Rgba::hex(0xFFFFFF), light).alpha(arrive(i) * shown)
        });

        // The anamorphic flare where the light is: a long blue streak and a bright core.
        let flare = smooth(0.55, 0.9, t) * (1.0 - smooth(1.5, 1.9, t)) * shown;
        if flare > 0.01 {
            let half = text.measure(r, WORD, INTRO) * scale * 0.5;
            let fx = cx - half + (2.0 * half) * sweep.clamp(0.0, 1.0);
            let fy = baseline - 70.0 * scale;
            r.shadow(Rect::new(fx - 760.0, fy - 2.0, 1520.0, 4.0), 2.0, 26.0, Rgba::hex(0x5E9BFF).alpha(0.55 * flare));
            r.rect(Rect::new(fx - 520.0, fy - 1.0, 1040.0, 2.0), 1.0, Rgba::hex(0xCFE2FF).alpha(0.45 * flare));
            r.shadow(Rect::new(fx - 18.0, fy - 18.0, 36.0, 36.0), 18.0, 40.0, Rgba::hex(0xFFFFFF).alpha(0.8 * flare));
        }

        // A hairline opens under the word.
        let line = smooth(1.0, 1.7, t);
        if line > 0.01 {
            let w = 560.0 * line * scale;
            r.rect(Rect::new(cx - w * 0.5, baseline + 64.0 * scale, w, 2.0), 1.0, Rgba::hex(0xAFC4E6).alpha(0.55 * word));
        }
    }
}
