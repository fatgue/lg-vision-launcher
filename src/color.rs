/// Straight-alpha sRGB colour in 0..1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const fn hex(rgb: u32) -> Self {
        Self {
            r: ((rgb >> 16) & 0xFF) as f32 / 255.0,
            g: ((rgb >> 8) & 0xFF) as f32 / 255.0,
            b: (rgb & 0xFF) as f32 / 255.0,
            a: 1.0,
        }
    }

    pub fn from_u8(rgb: [u8; 3]) -> Self {
        Self {
            r: rgb[0] as f32 / 255.0,
            g: rgb[1] as f32 / 255.0,
            b: rgb[2] as f32 / 255.0,
            a: 1.0,
        }
    }

    /// Parses `#RRGGBB`.
    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.trim().strip_prefix('#')?;
        (hex.len() == 6)
            .then(|| u32::from_str_radix(hex, 16).ok())
            .flatten()
            .map(Self::hex)
    }

    pub const fn alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    pub fn fade(self, factor: f32) -> Self {
        Self {
            a: self.a * factor,
            ..self
        }
    }

    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
            a: self.a + (other.a - self.a) * t,
        }
    }

    pub fn to_u8(self) -> [u8; 4] {
        let channel = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        [channel(self.r), channel(self.g), channel(self.b), channel(self.a)]
    }

    /// Scales so the brightest channel reaches `peak`, keeping hue: icon colours become
    /// luminous ambient light without turning grey.
    pub fn luminous(self, peak: f32) -> Self {
        let max = self.r.max(self.g).max(self.b).max(1e-3);
        let k = peak / max;
        Self {
            r: self.r * k,
            g: self.g * k,
            b: self.b * k,
            a: self.a,
        }
    }

    pub fn distance(self, other: Self) -> f32 {
        (self.r - other.r).abs() + (self.g - other.g).abs() + (self.b - other.b).abs() + (self.a - other.a).abs()
    }
}
