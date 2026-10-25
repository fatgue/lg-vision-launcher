/// Critically damped spring. Settles without overshoot, which reads calmer on a TV.
#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub value: f32,
    pub velocity: f32,
    pub target: f32,
    /// Angular frequency in rad/s; higher is snappier.
    pub omega: f32,
}

impl Spring {
    pub const fn new(value: f32, omega: f32) -> Self {
        Self { value, velocity: 0.0, target: value, omega }
    }

    pub fn snap(&mut self) {
        self.value = self.target;
        self.velocity = 0.0;
    }

    pub fn settled(&self) -> bool {
        self.value == self.target && self.velocity == 0.0
    }

    /// Advances by `dt` seconds using the exact critically damped solution (stable at any dt).
    /// Returns true while still moving.
    pub fn step(&mut self, dt: f32) -> bool {
        if self.settled() {
            return false;
        }
        let x0 = self.value - self.target;
        let w = self.omega;
        let e = (-w * dt).exp();
        let c = self.velocity + w * x0;
        let x = (x0 + c * dt) * e;
        self.velocity = (c - w * (x0 + c * dt)) * e;
        self.value = self.target + x;
        // Snap once well below a visible pixel/opacity step.
        let scale = 1.0f32.max(self.target.abs());
        if x.abs() < 1e-3 * scale && self.velocity.abs() < 1e-2 * scale {
            self.snap();
            return false;
        }
        true
    }
}
