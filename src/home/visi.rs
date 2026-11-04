//! Visi, the drone: after a while without input on Home it flies in and wrecks the place —
//! shoves tiles off the dock, juggles them, lasers holes into the screen and flies through
//! them, burns the clock, cracks the TV's glass, spins up a tornado — then rewinds everything
//! and leaves. Purely visual: the tiles' order and data never change. Any key or pointer
//! movement ends the show at once.
//!
//! The drone is drawn from eight views around it (`assets/visi.png`), blended while it turns;
//! rotors, lens, claw, thrust, beams and every effect are drawn live. Loose tiles and their
//! pieces are rigid bodies: they fall, land on their corners, bounce and tip over. Everything
//! lives in fixed-size pools, so a running show allocates next to nothing per frame.
use super::*;
use std::collections::HashMap;
use std::f32::consts::{PI, TAU};

const SPRITE_PNG: &[u8] = include_bytes!("../../assets/visi.png");
/// Eight views in a row, one every 45° as it turns: 7 faces the viewer, 1 looks left,
/// 4 shows its back, 6 looks right.
const VIEWS: usize = 8;
const CELL: f32 = 256.0;
const STEP: f32 = TAU / VIEWS as f32;
const FACE_FRONT: f32 = 7.0 * STEP;
const FACE_LEFT: f32 = 1.0 * STEP;
const FACE_RIGHT: f32 = 6.0 * STEP;
const FACE_AWAY: f32 = 4.0 * STEP;
/// The views' common height in the atlas (all share their top edge).
const REF: f32 = 206.0;

/// One view in the atlas: the sprite's size in its cell, the rotor hubs, the lens (None when
/// it looks away) and the claw, as fractions of the sprite.
struct View {
    w: f32,
    h: f32,
    hubs: [(f32, f32); 2],
    eye: Option<(f32, f32)>,
    claw: (f32, f32),
}

const VIEW_DATA: [View; VIEWS] = [
    View { w: 238.0, h: 172.0, hubs: [(0.10, 0.08), (0.87, 0.05)], eye: Some((0.31, 0.51)), claw: (0.5, 1.12) },
    View { w: 187.0, h: 205.0, hubs: [(0.14, 0.07), (0.77, 0.04)], eye: Some((0.12, 0.45)), claw: (0.42, 0.93) },
    View { w: 181.0, h: 206.0, hubs: [(0.35, 0.03), (0.62, 0.02)], eye: None, claw: (0.46, 0.93) },
    View { w: 198.0, h: 205.0, hubs: [(0.14, 0.03), (0.89, 0.06)], eye: None, claw: (0.53, 0.94) },
    View { w: 248.0, h: 202.0, hubs: [(0.10, 0.08), (0.89, 0.08)], eye: None, claw: (0.5, 0.9) },
    View { w: 177.0, h: 203.0, hubs: [(0.40, 0.02), (0.63, 0.03)], eye: None, claw: (0.56, 0.92) },
    View { w: 183.0, h: 206.0, hubs: [(0.17, 0.04), (0.93, 0.08)], eye: Some((0.82, 0.45)), claw: (0.51, 0.92) },
    View { w: 250.0, h: 204.0, hubs: [(0.11, 0.06), (0.91, 0.07)], eye: Some((0.60, 0.44)), claw: (0.51, 0.92) },
];

/// The drone's height on screen at depth 1.
const HEIGHT: f32 = 176.0;
const MAX_PARTICLES: usize = 280;
const MAX_FRAGMENTS: usize = 120;
const MAX_CRACKS: usize = 140;
const MAX_INK: usize = 900;
const GRAVITY: f32 = 2600.0;
const FLOOR: f32 = LOGICAL_H - 6.0;

/// A rigid rectangle: centre, half size, angle, velocities. Collides with the floor and the
/// screen's sides by its corners (impulses with friction), so it lands, tips and rolls.
#[derive(Clone, Copy, Default)]
struct Body {
    x: f32,
    y: f32,
    hw: f32,
    hh: f32,
    a: f32,
    vx: f32,
    vy: f32,
    w: f32,
}

impl Body {
    /// Advances it; returns the hardest impact speed (0: none).
    fn step(&mut self, dt: f32) -> f32 {
        self.vy += GRAVITY * dt;
        self.x += self.vx * dt;
        self.y += self.vy * dt;
        self.a += self.w * dt;
        let inertia = ((2.0 * self.hw).powi(2) + (2.0 * self.hh).powi(2)) / 12.0;
        let (c, s) = (self.a.cos(), self.a.sin());
        let corners = [(-self.hw, -self.hh), (self.hw, -self.hh), (self.hw, self.hh), (-self.hw, self.hh)];
        let arm = |(lx, ly): (f32, f32)| (lx * c - ly * s, lx * s + ly * c);
        // Push out of the floor and the sides by the deepest corner first.
        let (mut floor, mut left, mut right) = (0.0f32, 0.0f32, 0.0f32);
        for &corner in &corners {
            let (rx, ry) = arm(corner);
            floor = floor.max(self.y + ry - FLOOR);
            left = left.max(-(self.x + rx));
            right = right.max(self.x + rx - LOGICAL_W);
        }
        self.y -= floor;
        self.x += left - right;
        let mut impact = 0.0f32;
        for &corner in &corners {
            let (rx, ry) = arm(corner);
            let (px, py) = (self.x + rx, self.y + ry);
            if py >= FLOOR - 0.5 {
                let (vcx, vcy) = (self.vx - self.w * ry, self.vy + self.w * rx);
                if vcy > 0.0 {
                    impact = impact.max(vcy);
                    let bounce = if vcy > 380.0 { 0.32 } else { 0.0 };
                    let j = (1.0 + bounce) * vcy / (1.0 + rx * rx / inertia);
                    self.vy -= j;
                    self.w -= j * rx / inertia;
                    let jt = (-vcx / (1.0 + ry * ry / inertia)).clamp(-0.6 * j, 0.6 * j);
                    self.vx += jt;
                    self.w -= ry * jt / inertia;
                }
            }
            for (side, wall) in [(1.0f32, px <= 0.5), (-1.0, px >= LOGICAL_W - 0.5)] {
                if !wall {
                    continue;
                }
                let vcx = self.vx - self.w * ry;
                if vcx * side < 0.0 {
                    let j = -1.4 * vcx * side / (1.0 + ry * ry / inertia);
                    self.vx += j * side;
                    self.w -= ry * j * side / inertia;
                }
            }
        }
        if floor > 0.0 {
            self.w *= (-2.0 * dt).exp();
            self.vx *= (-1.5 * dt).exp();
        }
        impact
    }
}

#[derive(Clone, Copy)]
pub(super) struct TileFx {
    /// Where the tile belongs.
    home: Rect,
    pub dx: f32,
    pub dy: f32,
    pub angle: f32,
    pub scale: f32,
    pub alpha: f32,
    /// 0..1: burnt by the laser.
    pub scorch: f32,
    /// 0..1: cracks spreading.
    pub crack: f32,
    /// Shattered, broken or eaten: its pieces (if any) are drawn instead.
    pub hidden: bool,
    /// Loose: a rigid body under gravity (its centre, not the offset).
    body: Option<Body>,
}

impl TileFx {
    fn rest(home: Rect) -> Self {
        Self { home, dx: 0.0, dy: 0.0, angle: 0.0, scale: 1.0, alpha: 1.0, scorch: 0.0, crack: 0.0, hidden: false, body: None }
    }

    pub fn place(&self, rect: Rect) -> Rect {
        rect.scale_about(self.scale, 0.5, 0.5).offset(self.dx, self.dy)
    }

    fn center(&self) -> (f32, f32) {
        (self.home.x + self.home.w * 0.5 + self.dx, self.home.y + self.home.h * 0.5 + self.dy)
    }

    /// Lets it go: from here on physics moves it.
    fn loose(&mut self, vx: f32, vy: f32, spin: f32) {
        let (x, y) = self.center();
        let (hw, hh) = (self.home.w * 0.5 * self.scale, self.home.h * 0.5 * self.scale);
        self.body = Some(Body { x, y, hw, hh, a: self.angle, vx, vy, w: spin });
    }

    fn sync(&mut self) {
        if let Some(b) = self.body {
            self.dx = b.x - (self.home.x + self.home.w * 0.5);
            self.dy = b.y - (self.home.y + self.home.h * 0.5);
            self.angle = b.a;
        }
    }

    /// On the way back from `a` to rest (t 0..1).
    fn rewind(a: Self, t: f32) -> Self {
        let l = |x: f32, y: f32| x + (y - x) * t;
        Self {
            home: a.home,
            dx: l(a.dx, 0.0),
            dy: l(a.dy, 0.0),
            angle: l(a.angle, 0.0),
            scale: l(a.scale, 1.0),
            alpha: l(a.alpha, 1.0),
            scorch: l(a.scorch, 0.0),
            crack: l(a.crack, 0.0),
            hidden: a.hidden && t < 1.0,
            body: None,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Beat {
    /// 0 from the side, 1 out of a hole, 2 out of the depth.
    Enter(u8),
    Push(usize),
    Grab(usize),
    Laser(usize),
    Rocket(usize),
    Suck(usize),
    Break(usize),
    Juggle(usize),
    Tornado(usize),
    Burn,
    Dive,
    Portal,
    Smash,
    Disco,
    Dizzy,
    Graffiti,
    Repair,
    /// 0 off to the side, 1 into a hole, 2 up into the depth.
    Exit(u8),
}

impl Beat {
    fn length(self) -> f32 {
        match self {
            Beat::Enter(_) => 3.0,
            Beat::Push(_) => 3.6,
            Beat::Grab(_) => 4.8,
            Beat::Laser(_) => 3.8,
            Beat::Rocket(_) => 3.4,
            Beat::Suck(_) => 3.8,
            Beat::Break(_) => 3.0,
            Beat::Juggle(_) => 5.6,
            Beat::Tornado(_) => 5.0,
            Beat::Burn => 4.2,
            Beat::Dive => 5.2,
            Beat::Portal => 4.2,
            Beat::Smash => 5.2,
            Beat::Disco => 4.6,
            Beat::Dizzy => 4.8,
            Beat::Graffiti => 4.8,
            Beat::Repair => 3.2,
            Beat::Exit(_) => 2.6,
        }
    }

    /// Acts on a tile (and maybe its neighbours).
    fn tile(self) -> bool {
        matches!(
            self,
            Beat::Push(_) | Beat::Grab(_) | Beat::Laser(_) | Beat::Rocket(_) | Beat::Suck(_) | Beat::Break(_) | Beat::Juggle(_) | Beat::Tornado(_)
        )
    }
}

#[derive(Clone, Copy)]
struct Fragment {
    key: (usize, usize),
    /// The piece of the tile, as fractions of it.
    part: [f32; 4],
    body: Body,
    /// Where it belongs (centre), for the rewind.
    home: (f32, f32),
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// Falls and shrinks.
    Spark,
    /// Rises and grows.
    Smoke,
    /// Rises, shrinks, yellow to red.
    Flame,
    /// A soft glow that fades in place.
    Glow,
    /// Flutters down.
    Confetti,
}

#[derive(Clone, Copy)]
struct Particle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    life: f32,
    max: f32,
    size: f32,
    color: Rgba,
    kind: Kind,
}

/// A hole lasered into the screen: cut along its rim, then open (black, a few stars).
#[derive(Clone, Copy)]
struct Hole {
    x: f32,
    y: f32,
    r: f32,
    target: f32,
    cut: f32,
}

struct Drone {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    /// Its acceleration, smoothed: it tilts into it like a quadcopter.
    ax: f32,
    goal: (f32, f32),
    /// How quickly it follows its goal.
    pull: f32,
    /// Depth: 1 normal, smaller far away, larger close to the glass.
    z: f32,
    z_goal: f32,
    z_rate: f32,
    /// Drawn behind the tiles (deep in the room).
    behind: bool,
    yaw: f32,
    /// Where it looks (None: where it flies).
    face: Option<f32>,
    /// Spinning on its axis (rad/s), overrides `face`.
    spin: f32,
    roll: f32,
    /// Extra roll (tumbling, lying on its side).
    roll_extra: f32,
    /// Out of power: falls and bounces.
    falling: bool,
    rotor: f32,
    rotor_speed: f32,
    /// 0 closed .. 1 open.
    claw: f32,
    /// 0..1: the lens charging; and its colour.
    glow: f32,
    tint: Rgba,
    beam: Option<((f32, f32), Rgba, f32)>,
    /// 0 gone (inside a hole), 1 shown.
    shown: f32,
    /// Squashed flat (against the glass, a burp).
    squash: f32,
}

pub(super) struct Visi {
    t: f32,
    rng: u32,
    beats: Vec<Beat>,
    beat: usize,
    beat_t: f32,
    /// What Visi can hit: the tiles on screen when the show began, and the clock.
    targets: Vec<(Rect, usize, usize)>,
    clock: Rect,
    drone: Drone,
    pub(super) fx: HashMap<(usize, usize), TileFx>,
    fragments: Vec<Fragment>,
    particles: Vec<Particle>,
    next_particle: usize,
    rings: Vec<(f32, f32, f32)>,
    rocket: Option<(f32, f32, f32, f32)>,
    holes: [Option<Hole>; 2],
    /// Cracks in the glass: segments (x0, y0, x1, y1) and their distance from the impact.
    cracks: Vec<[f32; 5]>,
    crack_t: f32,
    crack_at: (f32, f32),
    /// The clock burning away (0..1).
    burn: f32,
    /// Laser graffiti on the wall: points (x, y, age, pen down).
    ink: Vec<(f32, f32, f32, bool)>,
    /// Tiles in the tornado: (key, angle, radius).
    whirl: Vec<((usize, usize), f32, f32)>,
    juggles: u32,
    shake: f32,
    flash: f32,
    /// The state the repair rewinds from.
    rewind_from: Vec<((usize, usize), TileFx)>,
    rewind_frags: Vec<(f32, f32, f32)>,
    rewind_burn: f32,
    rewind_ink: usize,
    rewinding: f32,
    /// Dismissed: rewind and leave quickly.
    quick: bool,
    done: bool,
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The shortest turn from `a` to `b`.
fn turn(a: f32, b: f32) -> f32 {
    (b - a + PI).rem_euclid(TAU) - PI
}

const SPARK: Rgba = Rgba::hex(0xFFD08A);
const DUST: Rgba = Rgba::hex(0xC8CDD6);
const LASER: Rgba = Rgba::hex(0xFF3B5C);
const REPAIR: Rgba = Rgba::hex(0x55C8FF);
const LENS: Rgba = Rgba::hex(0x3FE0FF);

impl Visi {
    pub(super) fn new(targets: Vec<(Rect, usize, usize)>, clock: Rect, seed: u32) -> Option<Self> {
        if targets.len() < 3 {
            return None;
        }
        let mut visi = Self {
            t: 0.0,
            rng: seed.max(1),
            beats: Vec::new(),
            beat: 0,
            beat_t: 0.0,
            targets,
            clock,
            drone: Drone {
                x: LOGICAL_W + 260.0,
                y: 260.0,
                vx: -900.0,
                vy: 0.0,
                ax: 0.0,
                goal: (LOGICAL_W * 0.55, 420.0),
                pull: 5.0,
                z: 1.0,
                z_goal: 1.0,
                z_rate: 3.0,
                behind: false,
                yaw: FACE_LEFT,
                face: None,
                spin: 0.0,
                roll: 0.0,
                roll_extra: 0.0,
                falling: false,
                rotor: 0.0,
                rotor_speed: 1.0,
                claw: 0.0,
                glow: 0.0,
                tint: LENS,
                beam: None,
                shown: 1.0,
                squash: 0.0,
            },
            fx: HashMap::new(),
            fragments: Vec::with_capacity(MAX_FRAGMENTS),
            particles: Vec::with_capacity(MAX_PARTICLES),
            next_particle: 0,
            rings: Vec::with_capacity(8),
            rocket: None,
            holes: [None; 2],
            cracks: Vec::with_capacity(MAX_CRACKS),
            crack_t: 0.0,
            crack_at: (0.0, 0.0),
            burn: 0.0,
            ink: Vec::with_capacity(MAX_INK),
            whirl: Vec::with_capacity(3),
            juggles: 0,
            shake: 0.0,
            flash: 0.0,
            rewind_from: Vec::new(),
            rewind_frags: Vec::new(),
            rewind_burn: 0.0,
            rewind_ink: 0,
            rewinding: 0.0,
            quick: false,
            done: false,
        };
        visi.plan();
        let first = visi.beats[0];
        visi.begin(first);
        Some(visi)
    }

    /// An entrance, eight of the fifteen acts in a random order (tile acts on different
    /// tiles), the repair and an exit.
    fn plan(&mut self) {
        type Act = fn(usize) -> Beat;
        let mut acts: Vec<Act> = vec![
            Beat::Push,
            Beat::Grab,
            Beat::Laser,
            Beat::Rocket,
            Beat::Suck,
            Beat::Break,
            Beat::Juggle,
            Beat::Tornado,
            |_| Beat::Burn,
            |_| Beat::Dive,
            |_| Beat::Portal,
            |_| Beat::Smash,
            |_| Beat::Disco,
            |_| Beat::Dizzy,
            |_| Beat::Graffiti,
        ];
        for i in (1..acts.len()).rev() {
            let j = (self.rand() * (i + 1) as f32) as usize % (i + 1);
            acts.swap(i, j);
        }
        let enter = (self.rand() * 3.0) as u8 % 3;
        self.beats.push(Beat::Enter(enter));
        let mut used: Vec<usize> = Vec::new();
        for act in acts {
            if self.beats.len() > 8 {
                break;
            }
            let target = self.pick(&used);
            let beat = act(target);
            if beat == Beat::Burn && self.clock.w <= 0.0 {
                continue;
            }
            if beat.tile() {
                used.push(target);
                // The neighbours too (dominoes, the laser's row, the tornado).
                let mut at = target;
                for _ in 0..3 {
                    match self.neighbour(at) {
                        Some(next) => {
                            used.push(next);
                            at = next;
                        }
                        None => break,
                    }
                }
            }
            self.beats.push(beat);
        }
        self.beats.push(Beat::Repair);
        let exit = (self.rand() * 3.0) as u8 % 3;
        self.beats.push(Beat::Exit(exit));
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng & 0xFFFF) as f32 / 65535.0
    }

    /// -1..1.
    fn spread(&mut self) -> f32 {
        self.rand() * 2.0 - 1.0
    }

    /// A tile not used yet (any, if all were).
    fn pick(&mut self, used: &[usize]) -> usize {
        let free: Vec<usize> = (0..self.targets.len()).filter(|i| !used.contains(i)).collect();
        if free.is_empty() {
            return (self.rand() * self.targets.len() as f32) as usize % self.targets.len();
        }
        free[(self.rand() * free.len() as f32) as usize % free.len()]
    }

    /// The next tile in the same row, to the right.
    fn neighbour(&self, target: usize) -> Option<usize> {
        let (_, rail, index) = self.targets[target];
        self.targets.iter().position(|&(_, r, i)| r == rail && i == index + 1)
    }

    fn key(&self, target: usize) -> (usize, usize) {
        let (_, rail, index) = self.targets[target];
        (rail, index)
    }

    fn center(&self, target: usize) -> (f32, f32) {
        let (rect, rail, index) = self.targets[target];
        match self.fx.get(&(rail, index)) {
            Some(fx) => fx.center(),
            None => (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5),
        }
    }

    fn tile(&mut self, target: usize) -> &mut TileFx {
        let (rect, rail, index) = self.targets[target];
        self.fx.entry((rail, index)).or_insert_with(|| TileFx::rest(rect))
    }

    /// Any key: rewind quickly and fly off.
    pub(super) fn dismiss(&mut self) {
        if self.quick {
            return;
        }
        self.quick = true;
        if matches!(self.beats.get(self.beat), Some(Beat::Repair | Beat::Exit(_))) {
            return;
        }
        self.beats.truncate(self.beat + 1);
        self.beats.push(Beat::Repair);
        self.beats.push(Beat::Exit(0));
        self.beat += 1;
        self.beat_t = 0.0;
        self.begin(Beat::Repair);
    }

    /// True in the one frame in which the current beat passes `at` seconds.
    fn crossed(&self, at: f32, dt: f32) -> bool {
        self.beat_t >= at && self.beat_t - dt < at
    }

    // ---- Particles ---------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn emit(&mut self, x: f32, y: f32, vx: f32, vy: f32, life: f32, size: f32, color: Rgba, kind: Kind) {
        let p = Particle { x, y, vx, vy, life, max: life, size, color, kind };
        if self.particles.len() < MAX_PARTICLES {
            self.particles.push(p);
        } else {
            self.particles[self.next_particle] = p;
            self.next_particle = (self.next_particle + 1) % MAX_PARTICLES;
        }
    }

    fn burst(&mut self, x: f32, y: f32, count: usize, speed: f32, color: Rgba, kind: Kind) {
        for _ in 0..count {
            let a = self.rand() * TAU;
            let s = speed * (0.3 + 0.7 * self.rand());
            let life = 0.5 + 0.8 * self.rand();
            let size = match kind {
                Kind::Smoke => 18.0 + 30.0 * self.rand(),
                Kind::Glow => 10.0 + 14.0 * self.rand(),
                Kind::Confetti => 8.0 + 6.0 * self.rand(),
                _ => 2.0 + 4.0 * self.rand(),
            };
            let lift = if kind == Kind::Spark { 300.0 } else { 0.0 };
            self.emit(x, y, a.cos() * s, a.sin() * s - lift, life, size, color, kind);
        }
    }

    /// Breaks a tile into pieces (cols × rows) flying away from (ox, oy).
    fn shatter(&mut self, target: usize, cols: usize, rows: usize, force: f32, ox: f32, oy: f32) {
        let (rect, rail, index) = self.targets[target];
        let fx = *self.tile(target);
        let at = fx.place(rect);
        self.tile(target).hidden = true;
        let (c, s) = (fx.angle.cos(), fx.angle.sin());
        let (cx, cy) = (at.x + at.w * 0.5, at.y + at.h * 0.5);
        for row in 0..rows {
            for col in 0..cols {
                if self.fragments.len() >= MAX_FRAGMENTS {
                    return;
                }
                let part = [col as f32 / cols as f32, row as f32 / rows as f32, (col + 1) as f32 / cols as f32, (row + 1) as f32 / rows as f32];
                // The piece's centre, turned with the tile.
                let (lx, ly) = (at.w * ((part[0] + part[2]) * 0.5 - 0.5), at.h * ((part[1] + part[3]) * 0.5 - 0.5));
                let (x, y) = (cx + lx * c - ly * s, cy + lx * s + ly * c);
                let (dx, dy) = (x - ox, y - oy);
                let len = dx.hypot(dy).max(1.0);
                let push = force * (0.6 + 0.6 * self.rand());
                let (jx, jy, spin) = (self.spread() * 200.0, -300.0 * self.rand(), self.spread() * 12.0);
                let home = (rect.x + rect.w * (part[0] + part[2]) * 0.5, rect.y + rect.h * (part[1] + part[3]) * 0.5);
                self.fragments.push(Fragment {
                    key: (rail, index),
                    part,
                    home,
                    body: Body {
                        x,
                        y,
                        hw: at.w * (part[2] - part[0]) * 0.5,
                        hh: at.h * (part[3] - part[1]) * 0.5,
                        a: fx.angle,
                        vx: dx / len * push + jx,
                        vy: dy / len * push + jy,
                        w: spin,
                    },
                });
            }
        }
    }

    /// Cracks running out from where something hit the glass.
    fn crack_glass(&mut self, x: f32, y: f32) {
        self.cracks.clear();
        self.crack_t = 0.0;
        self.crack_at = (x, y);
        let rays = 13;
        let mut ends: Vec<Vec<(f32, f32, f32)>> = Vec::with_capacity(rays);
        for i in 0..rays {
            let mut a = i as f32 / rays as f32 * TAU + self.spread() * 0.2;
            let (mut px, mut py, mut dist) = (x, y, 0.0);
            let reach = 380.0 + 520.0 * self.rand();
            let mut points = Vec::with_capacity(8);
            while dist < reach && self.cracks.len() < MAX_CRACKS - 30 {
                let step = 50.0 + 70.0 * self.rand();
                a += self.spread() * 0.35;
                let (nx, ny) = (px + a.cos() * step, py + a.sin() * step);
                self.cracks.push([px, py, nx, ny, dist]);
                // A branch now and then.
                if self.rand() < 0.22 {
                    let b = a + self.spread() * 0.9;
                    let len = 40.0 + 80.0 * self.rand();
                    self.cracks.push([nx, ny, nx + b.cos() * len, ny + b.sin() * len, dist + step]);
                }
                points.push((nx, ny, dist));
                px = nx;
                py = ny;
                dist += step;
            }
            ends.push(points);
        }
        // Rings around the impact: neighbouring rays joined at the same distance.
        for i in 0..rays {
            let j = (i + 1) % rays;
            for k in [0usize, 1] {
                if let (Some(&(ax, ay, d)), Some(&(bx, by, _))) = (ends[i].get(k), ends[j].get(k)) {
                    if self.cracks.len() < MAX_CRACKS {
                        self.cracks.push([ax, ay, bx, by, d]);
                    }
                }
            }
        }
    }

    // ---- Time ----------------------------------------------------------------------------

    /// Advances the show; false once Visi has left.
    pub(super) fn step(&mut self, dt: f32) -> bool {
        if self.done {
            return false;
        }
        self.t += dt;
        self.beat_t += dt;
        let Some(&beat) = self.beats.get(self.beat) else {
            self.done = true;
            return false;
        };
        let length = match beat {
            Beat::Repair if self.quick => 0.5,
            Beat::Exit(_) if self.quick => 1.2,
            _ => beat.length(),
        };
        if self.beat_t >= length {
            self.finish(beat);
            self.beat += 1;
            self.beat_t = 0.0;
            match self.beats.get(self.beat) {
                Some(&next) => self.begin(next),
                None => {
                    self.done = true;
                    return false;
                }
            }
        }
        if let Some(&beat) = self.beats.get(self.beat) {
            self.act(beat, dt);
        }
        self.physics(dt);
        true
    }

    fn begin(&mut self, beat: Beat) {
        let d = &mut self.drone;
        d.face = None;
        d.spin = 0.0;
        d.z_rate = 3.0;
        d.pull = 5.0;
        match beat {
            Beat::Enter(1) => {
                d.shown = 0.0;
                d.x = LOGICAL_W * 0.7;
                d.y = 330.0;
                d.vx = 0.0;
                d.goal = (d.x, d.y);
                d.z = 0.3;
                d.z_goal = 0.3;
                d.yaw = FACE_FRONT;
            }
            Beat::Enter(2) => {
                d.behind = true;
                d.x = LOGICAL_W * 0.3;
                d.y = 240.0;
                d.vx = 0.0;
                d.z = 0.22;
                d.goal = (d.x, d.y);
                d.yaw = FACE_FRONT;
            }
            Beat::Repair => {
                // Pieces fly back; an eaten tile (no pieces) grows back out of the drone.
                let keys: Vec<(usize, usize)> = self.fragments.iter().map(|f| f.key).collect();
                self.rewind_from = self
                    .fx
                    .iter()
                    .map(|(k, v)| {
                        let mut v = *v;
                        v.sync();
                        v.hidden &= keys.contains(k);
                        (*k, v)
                    })
                    .collect();
                self.rewind_frags = self.fragments.iter().map(|f| (f.body.x, f.body.y, f.body.a)).collect();
                self.rewind_burn = self.burn;
                self.rewind_ink = self.ink.len();
                self.rewinding = 0.0;
                self.whirl.clear();
                self.rocket = None;
                let d = &mut self.drone;
                d.beam = None;
                d.claw = 0.0;
                d.falling = false;
                d.roll_extra = 0.0;
                d.squash = 0.0;
                d.rotor_speed = 1.0;
                d.shown = 1.0;
                d.behind = false;
                d.z_goal = 1.0;
                for hole in self.holes.iter_mut().flatten() {
                    hole.target = 0.0;
                }
            }
            _ => {}
        }
    }

    fn finish(&mut self, beat: Beat) {
        let d = &mut self.drone;
        d.beam = None;
        d.claw = 0.0;
        d.glow = 0.0;
        d.tint = LENS;
        match beat {
            Beat::Repair => {
                self.fx.clear();
                self.fragments.clear();
                self.cracks.clear();
                self.ink.clear();
                self.burn = 0.0;
                self.rewinding = 0.0;
                self.holes = [None; 2];
            }
            Beat::Exit(_) => self.done = true,
            Beat::Disco => {
                for fx in self.fx.values_mut() {
                    if fx.body.is_none() && !fx.hidden {
                        fx.dy = 0.0;
                        fx.scale = 1.0;
                    }
                }
            }
            Beat::Dizzy => {
                d.falling = false;
                d.roll_extra = 0.0;
                d.rotor_speed = 1.0;
            }
            Beat::Smash => {
                d.squash = 0.0;
                d.behind = false;
                d.z_goal = 1.0;
            }
            Beat::Dive | Beat::Enter(_) => {
                d.behind = false;
                d.shown = 1.0;
                d.z_goal = 1.0;
            }
            Beat::Portal => {
                d.shown = 1.0;
                d.roll_extra = 0.0;
                d.z_goal = 1.0;
            }
            _ => {}
        }
    }

    /// What the current act does at `beat_t`.
    fn act(&mut self, beat: Beat, dt: f32) {
        let t = self.beat_t;
        match beat {
            Beat::Enter(kind) => self.enter(kind, t, dt),
            Beat::Push(target) => self.push(target, t, dt),
            Beat::Grab(target) => self.grab(target, t, dt),
            Beat::Laser(target) => self.laser(target, t, dt),
            Beat::Rocket(target) => self.rocket(target, t),
            Beat::Suck(target) => self.suck(target, t, dt),
            Beat::Break(target) => self.headbutt(target, t, dt),
            Beat::Juggle(target) => self.juggle(target, t, dt),
            Beat::Tornado(target) => self.tornado(target, t, dt),
            Beat::Burn => self.burn_clock(t),
            Beat::Dive => self.dive(t),
            Beat::Portal => self.portal(t, dt),
            Beat::Smash => self.smash(t, dt),
            Beat::Disco => self.disco(t),
            Beat::Dizzy => self.dizzy(t, dt),
            Beat::Graffiti => self.graffiti(t),
            Beat::Repair => self.repair(dt),
            Beat::Exit(kind) => self.exit(kind, t, dt),
        }
    }

    // ---- Acts ----------------------------------------------------------------------------

    fn enter(&mut self, kind: u8, t: f32, dt: f32) {
        let patrol = (LOGICAL_W * 0.5 + 260.0 * (t * 1.3).sin(), 380.0 + 40.0 * (t * 2.0).cos());
        match kind {
            1 => {
                // Out of a hole lasered from the other side.
                if self.crossed(0.2, dt) {
                    let (x, y) = (self.drone.x, self.drone.y);
                    self.holes[0] = Some(Hole { x, y, r: 0.0, target: 120.0, cut: 1.0 });
                    self.burst(x, y, 30, 520.0, SPARK, Kind::Spark);
                    self.shake = self.shake.max(8.0);
                }
                if self.crossed(0.7, dt) {
                    let d = &mut self.drone;
                    d.shown = 1.0;
                    d.z_goal = 1.0;
                    d.z_rate = 4.0;
                    d.vx = -500.0;
                    d.vy = 300.0;
                }
                if t > 0.9 {
                    self.drone.goal = patrol;
                    self.drone.pull = 3.0;
                }
                if self.crossed(1.8, dt) {
                    if let Some(hole) = &mut self.holes[0] {
                        hole.target = 0.0;
                    }
                }
            }
            2 => {
                // Out of the depth of the room, past the tiles.
                let d = &mut self.drone;
                d.z_goal = 1.0;
                d.z_rate = 1.1;
                d.face = Some(FACE_FRONT);
                d.goal = (LOGICAL_W * 0.45 + 200.0 * (t * 1.1).sin(), 300.0 + 60.0 * t);
                d.pull = 3.0;
                if d.z > 0.85 {
                    d.behind = false;
                }
            }
            _ => {
                self.drone.goal = patrol;
                self.drone.pull = 3.5;
            }
        }
    }

    fn push(&mut self, target: usize, t: f32, dt: f32) {
        let (cx, cy) = self.center(target);
        let w = self.targets[target].0.w;
        let d = &mut self.drone;
        d.face = Some(FACE_RIGHT);
        if t < 0.9 {
            d.goal = (cx - w * 0.5 - 150.0, cy - 30.0);
            d.pull = 5.0;
        } else if t < 1.2 {
            d.goal = (cx - w * 0.3, cy - 10.0);
            d.pull = 22.0;
        } else {
            d.goal = (cx - w * 0.5 - 140.0, cy - 160.0);
            d.pull = 5.0;
        }
        if self.crossed(1.05, dt) {
            self.shake = self.shake.max(10.0);
            self.burst(cx - w * 0.5, cy, 14, 260.0, DUST, Kind::Spark);
            self.tile(target).loose(620.0, -380.0, 2.2);
        }
        // Domino: it knocks into its neighbour, which goes over too.
        if self.crossed(1.35, dt) {
            if let Some(next) = self.neighbour(target) {
                let (nx, ny) = self.center(next);
                let nw = self.targets[next].0.w;
                self.burst(nx - nw * 0.5, ny, 10, 200.0, DUST, Kind::Spark);
                self.shake = self.shake.max(6.0);
                self.tile(next).loose(380.0, -260.0, -1.4);
            }
        }
    }

    fn grab(&mut self, target: usize, t: f32, dt: f32) {
        let (rect, _, _) = self.targets[target];
        let (cx, _) = self.center(target);
        let claw = self.claw_offset();
        let d = &mut self.drone;
        d.face = Some(FACE_FRONT);
        if t < 1.0 {
            d.goal = (cx - claw.0, rect.y + 6.0 - claw.1);
            d.pull = 6.0;
            d.claw = (t * 2.0).min(1.0);
        } else if t < 2.8 {
            d.claw = 0.2;
            d.face = None;
            let a = (t - 1.0) * 3.4;
            d.goal = (LOGICAL_W * 0.5 + 420.0 * a.sin(), 260.0 - 120.0 * (a * 0.5).sin());
            d.pull = 4.5;
            // It hangs from the claw, swinging.
            let (hx, hy) = (d.x + claw.0, d.y + claw.1 + rect.h * 0.5 - 6.0);
            let swing = 0.25 * (t * 6.0).sin() - d.vx * 0.0004;
            let fx = self.tile(target);
            fx.dx = hx - (rect.x + rect.w * 0.5);
            fx.dy = hy - (rect.y + rect.h * 0.5);
            fx.angle = swing;
            fx.scale = 0.92;
        } else if self.crossed(2.8, dt) {
            let (vx, vy) = (self.drone.vx * 0.6, self.drone.vy.max(0.0));
            let d = &mut self.drone;
            d.claw = 1.0;
            d.goal = (d.x + 60.0, d.y - 80.0);
            let spin = if vx >= 0.0 { 2.5 } else { -2.5 };
            let fx = self.tile(target);
            fx.scale = 1.0;
            fx.loose(vx, vy, spin);
        }
    }

    fn laser(&mut self, target: usize, t: f32, dt: f32) {
        let mut row = [target; 4];
        let mut count = 1;
        while count < 4 {
            match self.neighbour(row[count - 1]) {
                Some(next) => {
                    row[count] = next;
                    count += 1;
                }
                None => break,
            }
        }
        let (fx0, fy0) = self.center(row[0]);
        let (fx1, _) = self.center(row[count - 1]);
        let d = &mut self.drone;
        d.goal = ((fx0 + fx1) * 0.5, fy0 - 330.0);
        d.pull = 4.0;
        d.face = Some(FACE_FRONT);
        d.glow = (t / 0.9).min(1.0);
        d.tint = LASER;
        if (0.9..3.0).contains(&t) {
            let p = ((t - 0.9) / 1.9).clamp(0.0, 1.0);
            let bx = fx0 - 80.0 + (fx1 - fx0 + 160.0) * p;
            let by = fy0 + 20.0 * (t * 30.0).sin();
            d.beam = Some(((bx, by), LASER, 1.0));
            self.shake = self.shake.max(3.0);
            self.scorch_at(bx, by);
            for &i in &row[..count] {
                let (rect, _, _) = self.targets[i];
                let (cx, _) = self.center(i);
                if (bx - cx).abs() < rect.w * 0.6 {
                    let fx = self.tile(i);
                    fx.scorch = (fx.scorch + dt * 1.6).min(1.0);
                }
            }
        } else {
            d.beam = None;
            if t > 3.0 {
                d.tint = LENS;
                d.glow = 0.0;
            }
        }
    }

    /// Sparks and smoke where a laser burns.
    fn scorch_at(&mut self, x: f32, y: f32) {
        for _ in 0..2 {
            let (vx, vy, life) = (self.spread() * 350.0, -200.0 - 500.0 * self.rand(), 0.4 + 0.4 * self.rand());
            self.emit(x, y, vx, vy, life, 3.0, SPARK, Kind::Spark);
        }
        if self.rand() < 0.35 {
            let vx = self.spread() * 20.0;
            self.emit(x, y, vx, -60.0, 1.6, 24.0, Rgba::hex(0x2A2A2E), Kind::Smoke);
        }
    }

    fn rocket(&mut self, target: usize, t: f32) {
        let (cx, cy) = self.center(target);
        let d = &mut self.drone;
        let right = cx < LOGICAL_W * 0.5;
        d.goal = (if right { cx + 520.0 } else { cx - 520.0 }, cy - 300.0);
        d.face = Some(if right { FACE_LEFT } else { FACE_RIGHT });
        d.pull = 4.0;
        let (ex, ey) = self.eye();
        if (0.8..1.45).contains(&t) {
            let p = (t - 0.8) / 0.65;
            let (x, y) = (ex + (cx - ex) * p, ey + (cy - ey) * p - 120.0 * (p * PI).sin());
            self.rocket = Some((x, y, cx - ex, cy - ey));
            let (vx, vy) = (self.spread() * 40.0, self.spread() * 40.0);
            self.emit(x, y, vx, vy, 0.9, 16.0, Rgba::hex(0x8C8F96), Kind::Smoke);
            self.emit(x, y, -(cx - ex) * 0.3, -(cy - ey) * 0.3, 0.25, 7.0, Rgba::hex(0xFFB44A), Kind::Glow);
        } else if self.rocket.is_some() && t >= 1.45 {
            self.rocket = None;
            self.flash = 1.0;
            self.shake = self.shake.max(26.0);
            self.rings.push((cx, cy, 0.0));
            self.burst(cx, cy, 70, 900.0, Rgba::hex(0xFFC35A), Kind::Spark);
            self.burst(cx, cy, 30, 600.0, Rgba::hex(0xFF6A2A), Kind::Glow);
            self.burst(cx, cy, 14, 120.0, Rgba::hex(0x3A3A40), Kind::Smoke);
            self.shatter(target, 4, 3, 1100.0, cx, cy + 30.0);
            // The blast knocks the neighbour over.
            if let Some(next) = self.neighbour(target) {
                self.tile(next).loose(520.0, -700.0, 3.0);
            }
        }
    }

    fn suck(&mut self, target: usize, t: f32, dt: f32) {
        let (rect, _, _) = self.targets[target];
        let (cx, cy) = (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5);
        let claw = self.claw_offset();
        let d = &mut self.drone;
        d.goal = (cx - claw.0, rect.y - 210.0);
        d.pull = 5.0;
        d.glow = 0.6;
        d.face = Some(FACE_FRONT);
        d.claw = if (0.8..2.8).contains(&t) { 1.0 } else { 0.0 };
        if (0.8..2.8).contains(&t) {
            let p = ((t - 0.8) / 2.0).clamp(0.0, 1.0);
            let (hx, hy) = (d.x + claw.0, d.y + claw.1);
            let pull = p * p;
            let fx = self.tile(target);
            fx.dx = (hx - cx) * pull;
            fx.dy = (hy - cy) * pull;
            fx.scale = 1.0 - 0.97 * pull;
            fx.angle = 9.0 * pull;
            if p >= 0.97 {
                fx.hidden = true;
            }
            if self.rand() < 0.6 {
                let (sx, sy) = (cx + self.spread() * rect.w * 0.5, cy + self.spread() * rect.h * 0.5);
                self.emit(sx, sy, (hx - sx) * 1.6, (hy - sy) * 1.6, 0.6, 4.0, Rgba::hex(0xDDE3EC), Kind::Spark);
            }
            self.shake = self.shake.max(2.0);
        } else if self.crossed(3.0, dt) {
            // A burp: crumbs and a puff.
            let (hx, hy) = (self.drone.x, self.drone.y);
            self.flash = 0.25;
            self.drone.squash = 0.7;
            self.burst(hx, hy - 20.0, 26, 380.0, Rgba::hex(0x7FE7FF), Kind::Glow);
            self.burst(hx, hy + 40.0, 18, 300.0, DUST, Kind::Spark);
        }
    }

    fn headbutt(&mut self, target: usize, t: f32, dt: f32) {
        let (rect, _, _) = self.targets[target];
        let (cx, cy) = self.center(target);
        let d = &mut self.drone;
        d.goal = (cx + 40.0 * (t * 9.0).sin(), rect.y - 150.0 + 50.0 * (t * 9.0).sin().abs());
        d.pull = 8.0;
        d.face = Some(FACE_FRONT);
        if t < 1.2 {
            let fx = self.tile(target);
            fx.crack = (t / 1.1).min(1.0);
            if (t * 3.0).fract() < dt * 3.0 {
                self.shake = self.shake.max(7.0);
                self.burst(cx, rect.y, 8, 220.0, DUST, Kind::Spark);
            }
        } else if self.crossed(1.2, dt) {
            self.shake = self.shake.max(14.0);
            self.burst(cx, cy, 20, 300.0, DUST, Kind::Spark);
            self.shatter(target, 2, 1, 360.0, cx, cy - rect.h);
        }
    }

    fn juggle(&mut self, target: usize, t: f32, dt: f32) {
        let (rect, _, _) = self.targets[target];
        let (cx, _) = self.center(target);
        let key = self.key(target);
        if t < 0.8 {
            let d = &mut self.drone;
            d.goal = (cx, rect.y + rect.h + 130.0);
            d.pull = 6.0;
            d.face = Some(FACE_FRONT);
            return;
        }
        if self.crossed(0.8, dt) {
            self.juggles = 0;
            self.tile(target).loose(0.0, -1500.0, 1.5);
            self.shake = self.shake.max(8.0);
            self.burst(cx, rect.y + rect.h, 14, 300.0, DUST, Kind::Spark);
        }
        let Some(body) = self.fx.get(&key).and_then(|fx| fx.body) else { return };
        let height = HEIGHT * self.drone.z;
        if self.juggles < 4 && t < 4.8 {
            // Stays under it and heads it back up.
            let d = &mut self.drone;
            d.goal = (body.x, 600.0);
            d.pull = 7.0;
            let top = d.y - height * 0.5;
            if body.vy > 0.0 && body.y + body.hh >= top && (body.x - d.x).abs() < 150.0 {
                self.juggles += 1;
                let last = self.juggles == 4;
                let kick = (body.x - self.drone.x) * 4.0 + if last { 800.0 } else { 0.0 };
                let (vy, spin) = (-1250.0 - 250.0 * self.rand(), self.spread() * 5.0);
                if let Some(b) = self.fx.get_mut(&key).and_then(|fx| fx.body.as_mut()) {
                    b.vy = vy;
                    b.vx = kick;
                    b.w += spin;
                }
                self.drone.squash = 0.5;
                self.drone.vy += 300.0;
                self.shake = self.shake.max(5.0);
                self.burst(body.x, top, 12, 260.0, SPARK, Kind::Spark);
            }
        } else {
            let d = &mut self.drone;
            d.goal = (body.x - 380.0, 380.0);
            d.pull = 4.0;
        }
    }

    fn tornado(&mut self, target: usize, t: f32, dt: f32) {
        let (rect, _, _) = self.targets[target];
        let (ox, oy) = (rect.x + rect.w * 1.5, rect.y - 260.0);
        let d = &mut self.drone;
        d.goal = (ox, oy);
        d.pull = 5.0;
        d.face = Some(FACE_FRONT);
        if t < 0.8 {
            return;
        }
        if self.crossed(0.8, dt) {
            self.whirl.clear();
            let mut at = Some(target);
            for k in 0..3 {
                let Some(i) = at else { break };
                let key = self.key(i);
                self.tile(i);
                let (x, y) = self.center(i);
                let (dx, dy) = (self.drone.x, self.drone.y);
                self.whirl.push((key, (y - dy).atan2(x - dx) + k as f32 * 0.4, (x - dx).hypot(y - dy)));
                at = self.neighbour(i);
            }
        }
        let (dx, dy) = (self.drone.x, self.drone.y);
        if t < 3.4 {
            let ramp = smooth((t - 0.8) / 1.2);
            self.drone.spin = 16.0 * ramp;
            self.shake = self.shake.max(3.0 * ramp);
            // Wind: dust swirling around it.
            {
                let a = self.rand() * TAU;
                let r = 120.0 + 260.0 * self.rand();
                let (px, py) = (dx + a.cos() * r, dy + a.sin() * r * 0.45);
                self.emit(px, py, -a.sin() * 700.0, a.cos() * 300.0, 0.5, 6.0, DUST, Kind::Glow);
            }
            for k in 0..self.whirl.len() {
                let (key, a, r) = self.whirl[k];
                let a = a + dt * (2.0 + 7.0 * ramp);
                let r = r + (230.0 - r) * (1.0 - (-2.5 * dt).exp());
                self.whirl[k] = (key, a, r);
                let Some(fx) = self.fx.get_mut(&key) else { continue };
                let (tx, ty) = (dx + a.cos() * r, dy + a.sin() * r * 0.45);
                let (hx, hy) = (fx.home.x + fx.home.w * 0.5, fx.home.y + fx.home.h * 0.5);
                let k = 1.0 - (-5.0 * dt).exp();
                fx.dx += (tx - hx - fx.dx) * k;
                fx.dy += (ty - hy - fx.dy) * k;
                fx.angle += dt * 4.0 * ramp;
                // Seen in depth: smaller behind the drone.
                fx.scale = 0.78 + 0.18 * a.sin();
            }
        } else if self.crossed(3.4, dt) {
            // Let go: flung out, tumbling.
            for k in 0..self.whirl.len() {
                let (key, a, _) = self.whirl[k];
                let speed = 1100.0 + 300.0 * self.rand();
                if let Some(fx) = self.fx.get_mut(&key) {
                    fx.scale = 1.0;
                    fx.loose(-a.sin() * speed, a.cos() * speed * 0.45 - 500.0, 6.0);
                }
            }
            self.whirl.clear();
            self.shake = self.shake.max(10.0);
            self.burst(dx, dy, 30, 500.0, DUST, Kind::Glow);
        } else {
            self.drone.spin = (16.0 - (t - 3.4) * 12.0).max(0.0);
        }
    }

    fn burn_clock(&mut self, t: f32) {
        let c = self.clock;
        let left = c.x + c.w * 0.5 < LOGICAL_W * 0.5;
        let d = &mut self.drone;
        d.goal = (if left { c.x + c.w + 280.0 } else { c.x - 280.0 }, c.y + c.h + 180.0);
        d.pull = 4.0;
        d.face = Some(if left { FACE_LEFT } else { FACE_RIGHT });
        d.glow = (t / 0.9).min(1.0);
        d.tint = LASER;
        if (0.9..3.2).contains(&t) {
            let p = ((t - 0.9) / 2.1).clamp(0.0, 1.0);
            let bx = if left { c.x + c.w * p } else { c.x + c.w * (1.0 - p) };
            let by = c.y + c.h * (0.55 + 0.25 * (t * 23.0).sin());
            d.beam = Some(((bx, by), LASER, 1.0));
            self.burn = self.burn.max(p * 0.95);
            self.scorch_at(bx, by);
            // Flames along what already burns.
            for _ in 0..2 {
                let x = if left { c.x + c.w * p * self.rand() } else { c.x + c.w * (1.0 - p * self.rand()) };
                let y = c.y + c.h * (0.4 + 0.5 * self.rand());
                let (vx, vy, size) = (self.spread() * 30.0, -140.0 - 120.0 * self.rand(), 14.0 + 16.0 * self.rand());
                self.emit(x, y, vx, vy, 0.7, size, SPARK, Kind::Flame);
            }
        } else if t >= 3.2 {
            let d = &mut self.drone;
            d.beam = None;
            d.tint = LENS;
            d.glow = 0.0;
            self.burn = 1.0;
            if self.rand() < 0.4 {
                let x = c.x + c.w * self.rand();
                let vx = self.spread() * 15.0;
                self.emit(x, c.y + c.h * 0.6, vx, -50.0, 1.8, 26.0, Rgba::hex(0x2A2A2E), Kind::Smoke);
            }
        }
    }

    fn dive(&mut self, t: f32) {
        let d = &mut self.drone;
        if t < 1.3 {
            // Away into the room, behind the tiles.
            d.z_goal = 0.32;
            d.z_rate = 2.2;
            d.goal = (LOGICAL_W * 0.2, 300.0);
            d.pull = 2.5;
            d.face = Some(FACE_AWAY);
            if d.z < 0.75 {
                d.behind = true;
            }
        } else if t < 3.7 {
            // Across the back of the room, small, its lights in the distance.
            let p = (t - 1.3) / 2.4;
            d.goal = (LOGICAL_W * (0.15 + 0.7 * p), 330.0 + 120.0 * (p * PI * 2.0).sin());
            d.pull = 3.0;
            d.face = None;
        } else {
            // And back out towards the viewer.
            d.z_goal = 1.0;
            d.z_rate = 2.4;
            d.face = Some(FACE_FRONT);
            d.goal = (LOGICAL_W * 0.62, 420.0);
            if d.z > 0.85 {
                d.behind = false;
            }
        }
    }

    fn portal(&mut self, t: f32, dt: f32) {
        let (hx, hy) = (LOGICAL_W * 0.32, 380.0);
        let (ox, oy) = (LOGICAL_W * 0.78, 300.0);
        if t < 1.4 {
            // Cuts a round hole with its laser.
            if self.holes[0].is_none() {
                self.holes[0] = Some(Hole { x: hx, y: hy, r: 110.0, target: 110.0, cut: 0.0 });
            }
            let d = &mut self.drone;
            d.goal = (hx + 280.0, hy - 220.0);
            d.pull = 4.0;
            d.face = Some(FACE_FRONT);
            d.glow = 1.0;
            d.tint = LASER;
            let cut = ((t - 0.3) / 1.0).clamp(0.0, 1.0);
            if t > 0.3 {
                let a = -PI * 0.5 + cut * TAU;
                let (bx, by) = (hx + a.cos() * 110.0, hy + a.sin() * 110.0);
                d.beam = Some(((bx, by), LASER, 1.0));
                self.scorch_at(bx, by);
            }
            if let Some(hole) = &mut self.holes[0] {
                hole.cut = cut;
            }
            if self.crossed(1.3, dt) {
                self.burst(hx, hy, 24, 300.0, DUST, Kind::Spark);
                self.shake = self.shake.max(6.0);
            }
        } else if t < 2.2 {
            // Dives in, tumbling.
            let d = &mut self.drone;
            d.beam = None;
            d.tint = LENS;
            d.glow = 0.3;
            d.goal = (hx, hy);
            d.pull = 6.0;
            d.z_goal = 0.25;
            d.z_rate = 3.0;
            d.roll_extra += dt * 9.0;
            if t > 1.9 {
                d.shown = (1.0 - (t - 1.9) / 0.3).max(0.0);
            }
        } else if t < 2.6 {
            let d = &mut self.drone;
            d.shown = 0.0;
            d.roll_extra = 0.0;
            d.x = ox;
            d.y = oy;
            d.vx = 0.0;
            d.vy = 0.0;
            d.goal = (ox, oy);
            if let Some(hole) = &mut self.holes[0] {
                hole.target = 0.0;
            }
        } else {
            // Out of another hole somewhere else.
            if self.crossed(2.6, dt) {
                self.holes[1] = Some(Hole { x: ox, y: oy, r: 0.0, target: 120.0, cut: 1.0 });
                self.burst(ox, oy, 30, 520.0, SPARK, Kind::Spark);
                self.shake = self.shake.max(8.0);
            }
            if self.crossed(2.9, dt) {
                let d = &mut self.drone;
                d.shown = 1.0;
                d.z = 0.3;
                d.z_goal = 1.0;
                d.z_rate = 4.0;
                d.vx = -400.0;
                d.vy = 500.0;
            }
            let d = &mut self.drone;
            d.goal = (ox - 300.0, oy + 160.0);
            d.pull = 3.0;
            if t > 3.5 {
                if let Some(hole) = &mut self.holes[1] {
                    hole.target = 0.0;
                }
            }
        }
    }

    fn smash(&mut self, t: f32, dt: f32) {
        let (sx, sy) = (LOGICAL_W * 0.52, 430.0);
        let d = &mut self.drone;
        d.face = Some(FACE_FRONT);
        if t < 1.1 {
            // Back into the room for a run-up.
            d.z_goal = 0.4;
            d.z_rate = 3.0;
            d.goal = (sx, sy - 40.0);
            d.pull = 4.0;
            if d.z < 0.75 {
                d.behind = true;
            }
        } else if t < 1.75 {
            // Full speed at the glass.
            d.z_goal = 3.6;
            d.z_rate = 4.0;
            d.goal = (sx, sy);
            d.pull = 6.0;
            if d.z > 0.85 {
                d.behind = false;
            }
        } else if t < 3.2 {
            if self.crossed(1.75, dt) {
                self.drone.z = 3.3;
                let (x, y) = (self.drone.x, self.drone.y);
                self.crack_glass(x, y - 10.0);
                self.flash = 0.9;
                self.shake = 36.0;
                self.burst(x, y, 50, 900.0, Rgba::hex(0xEAF6FF), Kind::Spark);
            }
            // Squashed against the glass, sliding down it.
            let d = &mut self.drone;
            d.z_goal = 3.3;
            d.z_rate = 8.0;
            d.squash = 1.0;
            d.goal = (sx, sy + (t - 1.75) * 110.0);
            d.pull = 3.0;
            d.rotor_speed = 0.3;
            d.roll_extra = 0.08 * (t * 3.0).sin();
        } else {
            // Peels off and drops back into the room, shaking its head.
            let d = &mut self.drone;
            d.rotor_speed = 1.0;
            d.z_goal = 1.0;
            d.z_rate = 2.0;
            d.goal = (sx - 300.0, 360.0);
            d.pull = 3.0;
            d.roll_extra = 0.0;
            d.face = Some(FACE_FRONT + 0.7 * (t * 14.0).sin() * (1.0 - ((t - 3.2) / 1.6).min(1.0)));
        }
    }

    fn disco(&mut self, t: f32) {
        let d = &mut self.drone;
        d.goal = (LOGICAL_W * 0.5, 230.0 + 20.0 * (t * 4.0).sin());
        d.pull = 4.0;
        d.spin = 2.6;
        d.tint = hue_color(t * 1.5);
        d.glow = 0.6 + 0.4 * (t * 8.0).sin().abs();
        let pulse = (t * 2.0 * PI * 2.0).sin().max(0.0);
        for i in 0..self.targets.len() {
            let (home, rail, index) = self.targets[i];
            let fx = self.fx.entry((rail, index)).or_insert_with(|| TileFx::rest(home));
            if fx.body.is_some() || fx.hidden {
                continue;
            }
            let bob = (t * 2.0 * PI * 2.0 + i as f32 * 0.7).sin().max(0.0);
            fx.dy = -22.0 * bob;
            fx.scale = 1.0 + 0.04 * bob;
        }
        if self.rand() < 0.3 {
            let x = self.rand() * LOGICAL_W;
            let color = hue_color(self.rand());
            let (vx, vy) = (self.spread() * 60.0, 160.0 + 120.0 * self.rand());
            self.emit(x, -10.0, vx, vy, 4.0, 10.0, color, Kind::Confetti);
        }
        self.shake = self.shake.max(2.0 * pulse);
    }

    fn dizzy(&mut self, t: f32, dt: f32) {
        let height = HEIGHT * self.drone.z;
        if t < 1.3 {
            // Spins faster and faster…
            let d = &mut self.drone;
            d.goal = (LOGICAL_W * 0.6, 340.0);
            d.pull = 4.0;
            d.spin = 4.0 + 22.0 * t;
            d.roll_extra = 0.25 * (t * 9.0).sin() * t;
        } else if t < 3.5 {
            // …loses power and drops to the floor, bounces, lies there seeing stars.
            if self.crossed(1.3, dt) {
                let d = &mut self.drone;
                d.falling = true;
                d.spin = 6.0;
                d.glow = 0.0;
            }
            let d = &mut self.drone;
            d.rotor_speed = (d.rotor_speed - dt * 1.5).max(0.0);
            d.spin *= (-2.0 * dt).exp();
            let ground = FLOOR - height * 0.42;
            if d.y >= ground {
                d.y = ground;
                if d.vy > 300.0 {
                    let speed = d.vy;
                    d.vy = -speed * 0.38;
                    d.vx *= 0.5;
                    d.roll_extra += 0.5;
                    let (x, y) = (d.x, ground + height * 0.4);
                    self.shake = self.shake.max(speed / 60.0);
                    self.burst(x, y, 16, 260.0, DUST, Kind::Spark);
                } else {
                    d.vy = 0.0;
                    d.vx *= (-6.0 * dt).exp();
                    d.roll_extra += (1.15 - d.roll_extra) * (1.0 - (-6.0 * dt).exp());
                }
            }
        } else {
            // Reboots: the lens flickers, the rotors spin up, back in the air.
            let d = &mut self.drone;
            if d.falling {
                d.falling = false;
                d.vx = 0.0;
                d.vy = 0.0;
                d.goal = (d.x, d.y);
            }
            d.spin = 0.0;
            d.rotor_speed = ((t - 3.5) / 0.6).min(1.0);
            d.glow = if ((t - 3.5) * 12.0) as i32 % 2 == 0 { 1.0 } else { 0.1 };
            d.roll_extra *= (-4.0 * dt).exp();
            if t > 3.9 {
                d.goal = (d.x, 360.0);
                d.pull = 3.0;
            }
        }
    }

    fn graffiti(&mut self, t: f32) {
        let (gx, gy) = (LOGICAL_W * 0.7, 320.0);
        let d = &mut self.drone;
        d.goal = (gx - 400.0, gy - 110.0);
        d.pull = 3.5;
        d.face = Some(FACE_FRONT);
        d.glow = (t / 0.6).min(1.0);
        d.tint = LASER;
        if !(0.6..3.8).contains(&t) {
            d.beam = None;
            if t >= 3.8 {
                d.tint = LENS;
                d.glow = 0.0;
            }
            return;
        }
        // A smiley: the face, two eyes, the smile.
        let p = (t - 0.6) / 3.2;
        let (x, y) = if p < 0.5 {
            let a = -PI * 0.5 + p / 0.5 * TAU;
            (gx + a.cos() * 120.0, gy + a.sin() * 120.0)
        } else if p < 0.62 {
            let a = (p - 0.5) / 0.12 * TAU;
            (gx - 42.0 + a.cos() * 11.0, gy - 34.0 + a.sin() * 14.0)
        } else if p < 0.74 {
            let a = (p - 0.62) / 0.12 * TAU;
            (gx + 42.0 + a.cos() * 11.0, gy - 34.0 + a.sin() * 14.0)
        } else {
            let a = PI * (0.15 + 0.7 * (p - 0.74) / 0.26);
            (gx + a.cos() * 72.0, gy + 6.0 + a.sin() * 62.0)
        };
        d.beam = Some(((x, y), LASER, 1.0));
        // The pen lifts between the parts.
        // A point every 12 px is smooth enough; the pen lifts between the parts.
        let gap = self.ink.last().map_or(f32::MAX, |&(lx, ly, _, _)| (lx - x).hypot(ly - y));
        if gap >= 12.0 && self.ink.len() < MAX_INK {
            self.ink.push((x, y, 0.0, gap < 40.0));
        }
        self.scorch_at(x, y);
    }

    fn repair(&mut self, dt: f32) {
        // Rewind: every tile and piece back to where it belongs.
        let length = if self.quick { 0.4 } else { Beat::Repair.length() - 0.6 };
        self.rewinding = (self.rewinding + dt / length).min(1.0);
        let p = smooth(self.rewinding);
        let sweep = LOGICAL_W * (0.5 + 0.36 * (self.t * 3.0).sin());
        let d = &mut self.drone;
        d.goal = (LOGICAL_W * 0.5, 300.0);
        d.pull = 4.0;
        d.face = Some(FACE_FRONT);
        d.glow = 1.0 - p;
        d.tint = REPAIR;
        d.beam = (p < 0.95).then_some(((sweep, 760.0), REPAIR, 1.0 - p));
        for (key, from) in &self.rewind_from {
            self.fx.insert(*key, TileFx::rewind(*from, p));
        }
        for (f, &(x, y, a)) in self.fragments.iter_mut().zip(&self.rewind_frags) {
            f.body.x = x + (f.home.0 - x) * p;
            f.body.y = y + (f.home.1 - y) * p;
            f.body.a = a * (1.0 - p);
        }
        self.burn = self.rewind_burn * (1.0 - p);
        self.ink.truncate((self.rewind_ink as f32 * (1.0 - p)) as usize);
        if p >= 1.0 {
            self.fx.clear();
            self.fragments.clear();
            self.cracks.clear();
        }
    }

    fn exit(&mut self, kind: u8, t: f32, dt: f32) {
        let d = &mut self.drone;
        d.beam = None;
        d.glow = 0.0;
        d.tint = LENS;
        match if self.quick { 0 } else { kind } {
            1 => {
                // Into a hole of its own.
                let (hx, hy) = (LOGICAL_W * 0.85, 260.0);
                if self.crossed(0.1, dt) {
                    self.holes[0] = Some(Hole { x: hx, y: hy, r: 0.0, target: 110.0, cut: 1.0 });
                    self.burst(hx, hy, 24, 480.0, SPARK, Kind::Spark);
                }
                let d = &mut self.drone;
                d.goal = (hx, hy);
                d.pull = 4.0;
                if t > 0.9 {
                    d.z_goal = 0.25;
                    d.roll_extra += dt * 9.0;
                    d.shown = (1.0 - (t - 1.5) / 0.3).clamp(0.0, 1.0);
                }
                if t > 1.9 {
                    if let Some(hole) = &mut self.holes[0] {
                        hole.target = 0.0;
                    }
                }
            }
            2 => {
                // Up and away into the depth of the room.
                d.z_goal = 0.12;
                d.z_rate = 1.6;
                d.face = Some(FACE_AWAY);
                d.goal = (LOGICAL_W * 0.5, 120.0);
                d.pull = 2.0;
                if d.z < 0.75 {
                    d.behind = true;
                }
                d.shown = (1.0 - (t - 1.8) / 0.6).clamp(0.0, 1.0);
            }
            _ => {
                // A wave with the claw, and off to the side.
                d.claw = if (t * 6.0).sin() > 0.0 && t < 1.0 { 1.0 } else { 0.0 };
                d.face = if t < 1.0 { Some(FACE_FRONT) } else { None };
                d.goal = (-500.0, 140.0);
                d.pull = if t < 0.8 { 1.0 } else { 3.0 };
            }
        }
    }

    // ---- Physics -------------------------------------------------------------------------

    fn physics(&mut self, dt: f32) {
        let d = &mut self.drone;
        if d.falling {
            d.vy += GRAVITY * dt;
            d.ax *= (-4.0 * dt).exp();
        } else {
            // A quadcopter: it tilts into its acceleration.
            let (gx, gy) = d.goal;
            let k = d.pull * d.pull;
            let damp = 2.0 * d.pull;
            let ax = ((gx - d.x) * k - d.vx * damp).clamp(-5000.0, 5000.0);
            let ay = ((gy - d.y) * k - d.vy * damp).clamp(-5000.0, 5000.0);
            d.vx += ax * dt;
            d.vy += ay * dt;
            d.ax += (ax - d.ax) * (1.0 - (-8.0 * dt).exp());
        }
        d.x += d.vx * dt;
        d.y += d.vy * dt + if d.falling { 0.0 } else { (self.t * 3.1).sin() * 12.0 * dt };
        d.roll = (d.ax.atan2(GRAVITY) * 0.9).clamp(-0.6, 0.6);
        d.z += (d.z_goal - d.z) * (1.0 - (-d.z_rate * dt).exp());
        d.rotor += dt * 55.0 * d.rotor_speed;
        d.squash *= (-3.0 * dt).exp();
        // Turns to where it looks (or flies), or spins.
        if d.spin.abs() > 0.01 {
            d.yaw = (d.yaw + d.spin * dt).rem_euclid(TAU);
        } else {
            let want = d.face.unwrap_or(if d.vx > 140.0 {
                FACE_RIGHT
            } else if d.vx < -140.0 {
                FACE_LEFT
            } else {
                d.yaw
            });
            d.yaw = (d.yaw + turn(d.yaw, want) * (1.0 - (-5.0 * dt).exp())).rem_euclid(TAU);
        }
        // Thrust under the body.
        if !d.falling && d.shown > 0.5 && d.rotor_speed > 0.5 && self.rand() < 0.3 {
            let z = self.drone.z;
            let (x, y) = (self.drone.x + self.spread() * 25.0 * z, self.drone.y + HEIGHT * 0.5 * z);
            let vx = self.spread() * 15.0;
            let vy = (110.0 + self.drone.vy.max(0.0) * 0.2) * z;
            self.emit(x, y, vx, vy, 0.35, 12.0 * z, Rgba::hex(0x9FE8FF), Kind::Glow);
        }
        for p in &mut self.particles {
            if p.life <= 0.0 {
                continue;
            }
            p.life -= dt;
            match p.kind {
                Kind::Smoke => {
                    p.vy -= 40.0 * dt;
                    p.size += 26.0 * dt;
                }
                Kind::Flame => {
                    p.vy -= 160.0 * dt;
                    p.size *= (-1.2 * dt).exp();
                }
                Kind::Glow => {}
                Kind::Confetti => p.vx = 80.0 * (p.life * 5.0 + p.size).sin(),
                Kind::Spark => p.vy += GRAVITY * 0.55 * dt,
            }
            p.x += p.vx * dt;
            p.y += p.vy * dt;
        }
        if self.rewinding <= 0.0 {
            let mut impacts = [(0.0, 0.0, 0.0); 4];
            let mut count = 0;
            for fx in self.fx.values_mut() {
                if let Some(body) = &mut fx.body {
                    let hit = body.step(dt);
                    if hit > 520.0 && count < impacts.len() {
                        impacts[count] = (body.x, body.y + body.hh, hit);
                        count += 1;
                    }
                }
                fx.sync();
            }
            for f in &mut self.fragments {
                let hit = f.body.step(dt);
                if hit > 700.0 && count < impacts.len() {
                    impacts[count] = (f.body.x, f.body.y + f.body.hh, hit * 0.4);
                    count += 1;
                }
            }
            for &(x, y, hit) in &impacts[..count] {
                self.shake = self.shake.max((hit / 70.0).min(16.0));
                self.burst(x, y, 12, 300.0, DUST, Kind::Spark);
            }
            for ink in &mut self.ink {
                ink.2 += dt;
            }
        }
        if !self.cracks.is_empty() {
            self.crack_t += dt;
        }
        for hole in self.holes.iter_mut() {
            if let Some(h) = hole {
                h.r += (h.target - h.r) * (1.0 - (-9.0 * dt).exp());
                if h.target == 0.0 && h.r < 2.0 {
                    *hole = None;
                }
            }
        }
        for ring in &mut self.rings {
            ring.2 += dt;
        }
        self.rings.retain(|r| r.2 < 0.7);
        self.shake *= (-9.0 * dt).exp();
        self.flash *= (-7.0 * dt).exp();
    }

    // ---- Geometry ------------------------------------------------------------------------

    /// The two views to draw and how far it has turned from the first to the second.
    fn views(&self) -> (usize, usize, f32) {
        let f = self.drone.yaw.rem_euclid(TAU) / STEP;
        let first = f.floor() as usize % VIEWS;
        (first, (first + 1) % VIEWS, f.fract())
    }

    /// Screen pixels per atlas pixel.
    fn scale(&self) -> f32 {
        HEIGHT * self.drone.z / REF
    }

    /// A view's rect on screen (all views share their top edge).
    fn rect(&self, view: usize) -> Rect {
        let v = &VIEW_DATA[view];
        let s = self.scale();
        let (w, h) = (v.w * s, v.h * s);
        Rect::new(self.drone.x - w * 0.5, self.drone.y - REF * 0.5 * s, w, h)
    }

    /// A point of a view's sprite (fractions) on screen, ignoring roll.
    fn point(&self, view: usize, (fx, fy): (f32, f32)) -> (f32, f32) {
        let r = self.rect(view);
        (r.x + fx * r.w, r.y + fy * r.h)
    }

    /// The view it mostly shows.
    fn main_view(&self) -> usize {
        let (a, b, f) = self.views();
        if f < 0.5 {
            a
        } else {
            b
        }
    }

    fn eye(&self) -> (f32, f32) {
        let view = self.main_view();
        match VIEW_DATA[view].eye {
            Some(eye) => self.point(view, eye),
            None => (self.drone.x, self.drone.y),
        }
    }

    fn claw_offset(&self) -> (f32, f32) {
        let view = self.main_view();
        let (x, y) = self.point(view, VIEW_DATA[view].claw);
        (x - self.drone.x, y - self.drone.y)
    }

    /// Screen shake for this frame.
    pub(super) fn shake(&self) -> (f32, f32) {
        let s = self.shake;
        (s * (self.t * 71.0).sin(), s * (self.t * 53.0).cos())
    }

    pub(super) fn rect_of(&self, rail: usize, index: usize) -> Option<Rect> {
        self.targets.iter().find(|&&(_, r, i)| r == rail && i == index).map(|&(rect, _, _)| rect)
    }

    /// The clock: how much is left of it (it burns away).
    pub(super) fn clock_alpha(&self) -> f32 {
        1.0 - smooth(self.burn)
    }
}

fn hue_color(h: f32) -> Rgba {
    let h = h.rem_euclid(1.0) * 6.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    let (r, g, b) = match h as i32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    Rgba { r: 0.35 + 0.65 * r, g: 0.35 + 0.65 * g, b: 0.35 + 0.65 * b, a: 1.0 }
}

/// A line as a thin rotated rectangle (leaves no rotation set).
fn line(r: &mut Renderer, (x0, y0): (f32, f32), (x1, y1): (f32, f32), width: f32, color: Rgba) {
    let (dx, dy) = (x1 - x0, y1 - y0);
    let len = dx.hypot(dy);
    if len < 0.5 {
        return;
    }
    let (mx, my) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
    r.set_rotation(Some((mx, my, dy.atan2(dx))));
    r.rect(Rect::new(mx - len * 0.5 - width * 0.5, my - width * 0.5, len + width, width), width * 0.5, color);
    r.set_rotation(None);
}

impl Home {
    /// The effect on one tile during a show (None: as usual).
    pub(super) fn visi_tile(&self, rail: usize, index: usize) -> Option<TileFx> {
        self.visi.as_ref()?.fx.get(&(rail, index)).copied()
    }

    /// Places a tile with its effect (moved, turned, scaled); its alpha factor, or None for a
    /// tile drawn in pieces (by `draw_visi`) or gone.
    pub(super) fn visi_begin(&self, r: &mut Renderer, rail: usize, index: usize, rect: Rect) -> Option<(Rect, f32)> {
        let Some(fx) = self.visi_tile(rail, index) else { return Some((rect, 1.0)) };
        if fx.hidden {
            return None;
        }
        let placed = fx.place(rect);
        if fx.angle.abs() > 0.001 {
            r.set_rotation(Some((placed.x + placed.w * 0.5, placed.y + placed.h * 0.5, fx.angle)));
        }
        // Text keeps its size: a shrinking tile fades out instead.
        Some((placed, fx.alpha * ((fx.scale - 0.15) / 0.55).clamp(0.0, 1.0)))
    }

    /// The laser's burn and the cracks over a tile (last pass; `rect` is already placed).
    pub(super) fn visi_marks(&self, r: &mut Renderer, rail: usize, index: usize, rect: Rect, radius: f32) {
        let Some(fx) = self.visi_tile(rail, index) else { return };
        if fx.scorch > 0.01 {
            r.rect(rect, radius, Rgba::hex(0x120A06).alpha(0.75 * fx.scorch));
            r.stroke(rect.inflate(-1.0), radius, 3.0, Rgba::hex(0xFF7A2A).alpha(0.8 * fx.scorch));
        }
        if fx.crack > 0.01 {
            // A zig-zag from the top centre down (straight tiles only: it is drawn unrotated).
            let (cx, top) = (rect.x + rect.w * 0.5, rect.y);
            let mut prev = (cx, top);
            for k in 1..=6 {
                let f = k as f32 / 6.0;
                if f > fx.crack {
                    break;
                }
                let next = (cx + if k % 2 == 0 { -14.0 } else { 16.0 } * f, top + rect.h * f);
                line(r, prev, next, 3.0, Rgba::hex(0x0B0B0E).alpha(0.9));
                prev = next;
            }
        }
    }

    pub(super) fn visi_end(&self, r: &mut Renderer) {
        r.set_rotation(None);
    }

    /// The clock's alpha during a show (it may burn away).
    pub(super) fn visi_clock(&self) -> f32 {
        self.visi.as_ref().map_or(1.0, |v| v.clock_alpha())
    }

    /// What lies behind the tiles: the drone deep in the room, soot and laser graffiti.
    pub(super) fn draw_visi_back(&mut self, r: &mut Renderer) {
        if self.visi.is_none() {
            return;
        }
        self.load_visi_sprite(r);
        let Some(visi) = &self.visi else { return };
        if visi.burn > 0.01 {
            let c = visi.clock;
            r.shadow(c.inflate(-10.0), 30.0, 60.0, Rgba::hex(0x0A0806).alpha(0.55 * visi.burn));
            // Embers glowing in the soot while it burns.
            let glow = (visi.burn * (1.0 - visi.burn) * 4.0).min(1.0);
            r.shadow(c.inflate(-20.0), 30.0, 50.0, Rgba::hex(0xFF6A1A).alpha(0.35 * glow));
        }
        draw_ink(r, &visi.ink);
        if visi.drone.behind {
            self.draw_drone(r, visi, true);
        }
    }

    /// Holes, pieces, particles, the rocket, the drone, cracks in the glass, the flash and
    /// the rewind's scanlines.
    pub(super) fn draw_visi(&mut self, r: &mut Renderer) {
        let Some(visi) = &self.visi else { return };
        let white = Rgba::hex(0xFFFFFF);

        for hole in visi.holes.iter().flatten() {
            draw_hole(r, hole, visi.t);
        }

        // Pieces of broken tiles, each a cut-out of the tile.
        for f in &visi.fragments {
            let Some(rect) = visi.rect_of(f.key.0, f.key.1) else { continue };
            let Some(tile) = self.rails.get(f.key.0).and_then(|rail| rail.tiles.get(f.key.1)) else { continue };
            let b = f.body;
            let piece = Rect::new(b.x - b.hw, b.y - b.hh, b.hw * 2.0, b.hh * 2.0);
            r.set_rotation(Some((b.x, b.y, b.a)));
            r.shadow(piece.offset(0.0, 10.0), 6.0, 16.0, Rgba::hex(0).alpha(0.4));
            self.draw_tile_piece(r, tile, rect, f.part, piece);
            r.stroke(piece.inflate(-0.5), 3.0, 1.0, white.alpha(0.3));
            r.set_rotation(None);
        }

        for &(x, y, t) in &visi.rings {
            let radius = 40.0 + t * 900.0;
            let a = (1.0 - t / 0.7).max(0.0);
            r.stroke(Rect::new(x - radius, y - radius, radius * 2.0, radius * 2.0), radius, 6.0 * a + 1.0, Rgba::hex(0xFFE2B0).alpha(0.7 * a));
        }

        for p in &visi.particles {
            if p.life <= 0.0 {
                continue;
            }
            let k = p.life / p.max;
            let rect = |size: f32| Rect::new(p.x - size * 0.5, p.y - size * 0.5, size, size);
            match p.kind {
                Kind::Smoke => r.shadow(rect(p.size), p.size * 0.5, p.size * 0.8, p.color.alpha(0.35 * k)),
                Kind::Glow => {
                    let size = p.size * (0.5 + 0.5 * k);
                    r.shadow(rect(size), size * 0.5, size, p.color.alpha(0.8 * k));
                }
                Kind::Flame => {
                    let color = Rgba::hex(0xFF3A10).lerp(Rgba::hex(0xFFD86A), k);
                    r.shadow(rect(p.size), p.size * 0.5, p.size * 0.9, color.alpha(0.9 * k));
                }
                Kind::Confetti => {
                    r.set_rotation(Some((p.x, p.y, p.life * 7.0 + p.size)));
                    r.rect(Rect::new(p.x - p.size * 0.5, p.y - p.size * 0.25, p.size, p.size * 0.5), 1.0, p.color.alpha((k / 0.15).min(1.0)));
                    r.set_rotation(None);
                }
                Kind::Spark => {
                    let size = p.size * (0.4 + 0.6 * k);
                    r.rect(rect(size), size * 0.5, p.color.alpha(k));
                }
            }
        }

        if let Some((x, y, dx, dy)) = visi.rocket {
            r.set_rotation(Some((x, y, dy.atan2(dx))));
            r.rect(Rect::new(x - 22.0, y - 6.0, 44.0, 12.0), 6.0, Rgba::hex(0xD9DEE6));
            r.rect(Rect::new(x + 12.0, y - 6.0, 10.0, 12.0), 6.0, Rgba::hex(0xFF4D5E));
            r.set_rotation(None);
            r.shadow(Rect::new(x - 34.0, y - 10.0, 20.0, 20.0), 10.0, 22.0, Rgba::hex(0xFFB44A).alpha(0.9));
        }

        if !visi.drone.behind {
            self.draw_drone(r, visi, false);
        }

        // Cracks in the TV's glass, over everything, spreading from the impact.
        if !visi.cracks.is_empty() {
            let reach = visi.crack_t * 3800.0;
            let fade = 1.0 - smooth(visi.rewinding);
            let (ix, iy) = visi.crack_at;
            r.shadow(Rect::new(ix - 50.0, iy - 50.0, 100.0, 100.0), 50.0, 60.0, white.alpha(0.35 * fade));
            for c in &visi.cracks {
                if c[4] > reach {
                    continue;
                }
                let grow = ((reach - c[4]) / 120.0).min(1.0);
                let end = (c[0] + (c[2] - c[0]) * grow, c[1] + (c[3] - c[1]) * grow);
                let width = (3.2 - c[4] / 400.0).max(1.0);
                line(r, (c[0], c[1]), end, width, white.alpha(0.85 * fade));
            }
        }

        if visi.flash > 0.01 {
            r.rect(Rect::new(0.0, 0.0, LOGICAL_W, LOGICAL_H), 0.0, Rgba::hex(0xFFF3DA).alpha(0.75 * visi.flash));
        }
        // The rewind: scanlines and a cold tint.
        if visi.rewinding > 0.0 && visi.rewinding < 1.0 {
            let a = (visi.rewinding * PI).sin();
            r.rect(Rect::new(0.0, 0.0, LOGICAL_W, LOGICAL_H), 0.0, Rgba::hex(0x0A2A55).alpha(0.18 * a));
            let shift = (visi.t * 900.0) % 24.0;
            let mut y = -24.0 + shift;
            while y < LOGICAL_H {
                r.rect(Rect::new(0.0, y, LOGICAL_W, 2.0), 0.0, white.alpha(0.06 * a));
                y += 24.0;
            }
        }
    }

    /// The drone: its beam, a shadow, the body from the two nearest views (blended as it
    /// turns), rotors, claw and lens. Deep in the room it is darker.
    fn draw_drone(&self, r: &mut Renderer, visi: &Visi, behind: bool) {
        let d = &visi.drone;
        // Not before its sprite is ready (a few frames after the first show starts).
        if d.shown <= 0.01 || !matches!(self.visi_sprite, Some(Some(_))) {
            return;
        }
        let white = Rgba::hex(0xFFFFFF);
        let (ex, ey) = visi.eye();
        if let Some(((bx, by), color, a)) = d.beam {
            let mid = ((ex + bx) * 0.5, (ey + by) * 0.5);
            let len = (bx - ex).hypot(by - ey);
            r.set_rotation(Some((mid.0, mid.1, (by - ey).atan2(bx - ex))));
            r.shadow(Rect::new(mid.0 - len * 0.5, mid.1 - 9.0, len, 18.0), 9.0, 26.0, color.alpha(0.7 * a));
            r.rect(Rect::new(mid.0 - len * 0.5, mid.1 - 2.5, len, 5.0), 2.5, white.alpha(0.95 * a));
            r.set_rotation(None);
            r.shadow(Rect::new(bx - 20.0, by - 20.0, 40.0, 40.0), 20.0, 40.0, color.alpha(a));
        }
        let z = d.z;
        let fog = if behind { 0.55 + 0.45 * ((z - 0.2) / 0.6).clamp(0.0, 1.0) } else { 1.0 };
        let shade = Rgba { r: fog, g: fog, b: (fog * 1.04).min(1.0), a: d.shown };
        if !behind && !d.falling && z < 1.6 {
            r.shadow(Rect::new(d.x - 70.0 * z, d.y + HEIGHT * z * 0.9, 140.0 * z, 24.0 * z), 12.0 * z, 30.0 * z, Rgba::hex(0).alpha(0.22 * d.shown));
        }
        let (a, b, f) = visi.views();
        // A slight squeeze while it turns between two views reads as depth.
        let squeeze = 1.0 - 0.1 * (f * PI).sin();
        let squash = d.squash.min(1.0);
        r.set_rotation(Some((d.x, d.y, d.roll + d.roll_extra)));
        if let Some(Some(sprite)) = self.visi_sprite {
            for (view, alpha) in [(a, 1.0), (b, smooth((f - 0.2) / 0.6))] {
                if alpha <= 0.01 {
                    continue;
                }
                let v = &VIEW_DATA[view];
                let base = visi.rect(view);
                let (w, h) = (base.w * squeeze * (1.0 + 0.18 * squash), base.h * (1.0 - 0.14 * squash));
                let rect = Rect::new(d.x - w * 0.5, base.y + (base.h - h), w, h);
                let atlas_w = CELL * VIEWS as f32;
                let x0 = view as f32 * CELL;
                let uv = [(x0 + 0.5) / atlas_w, 0.5 / CELL, (x0 + v.w - 0.5) / atlas_w, (v.h - 0.5) / CELL];
                r.image(sprite, rect, uv, 0.0, shade.alpha(d.shown * alpha));
            }
        }
        // Rotors: a blurred disc and two blades whose visible length follows the turn.
        let view = visi.main_view();
        for (k, &hub) in VIEW_DATA[view].hubs.iter().enumerate() {
            let (hx, hy) = visi.point(view, hub);
            let hy = hy - 4.0 * z;
            let radius = HEIGHT * z * 0.27;
            r.rect(Rect::new(hx - radius, hy - radius * 0.14, radius * 2.0, radius * 0.28), radius * 0.14, Rgba::hex(0x2A2E36).alpha(0.22 * d.rotor_speed * d.shown));
            for blade in 0..2 {
                let angle = d.rotor + k as f32 * 1.3 + blade as f32 * PI;
                let reach = radius * angle.cos();
                let (x0, x1) = (hx.min(hx + reach), hx.max(hx + reach));
                let y = hy + radius * 0.1 * angle.sin();
                r.rect(Rect::new(x0, y - 3.0 * z, (x1 - x0).max(4.0 * z), 6.0 * z), 3.0 * z, Rgba::hex(0x24272E).alpha(0.85 * d.shown * fog));
            }
        }
        // The claw opens: two fingers under it.
        if d.claw > 0.05 {
            let (cx, cy) = visi.point(view, VIEW_DATA[view].claw);
            for side in [-1.0f32, 1.0] {
                let angle = side * (0.15 + 0.6 * d.claw);
                let len = 26.0 * z;
                let (tx, ty) = (cx + angle.sin() * len, cy + angle.cos() * len);
                line(r, (cx + side * 4.0 * z, cy), (tx, ty), 6.0 * z, Rgba::hex(0x22252B).alpha(d.shown * fog));
            }
            r.set_rotation(Some((d.x, d.y, d.roll + d.roll_extra)));
        }
        // The lens.
        if let Some(eye) = VIEW_DATA[view].eye {
            let (lx, ly) = visi.point(view, eye);
            let glow = 0.35 + 0.65 * d.glow;
            let size = 44.0 * z;
            r.shadow(Rect::new(lx - size * 0.5, ly - size * 0.5, size, size), size * 0.5, (30.0 + 40.0 * d.glow) * z, d.tint.alpha(0.55 * glow * d.shown * fog));
        }
        r.set_rotation(None);
        // Seeing stars after the crash.
        if d.falling || d.roll_extra > 0.6 {
            for k in 0..3 {
                let a = visi.t * 4.0 + k as f32 * TAU / 3.0;
                let (sx, sy) = (d.x + a.cos() * 60.0 * z, d.y - HEIGHT * z * 0.6 + a.sin() * 14.0 * z);
                r.shadow(Rect::new(sx - 6.0, sy - 6.0, 12.0, 12.0), 6.0, 10.0, Rgba::hex(0xFFE06A).alpha(0.9));
                r.rect(Rect::new(sx - 3.0, sy - 3.0, 6.0, 6.0), 3.0, white);
            }
        }
    }

    /// One piece of a broken tile: its part of the fill, the artwork or the icon.
    fn draw_tile_piece(&self, r: &mut Renderer, tile: &Tile, rect: Rect, part: [f32; 4], piece: Rect) {
        let art = self.arts.get(&tile.id).map(|(_, uv)| *uv);
        let edge = self.blended.get(&tile.id).copied();
        let fill = match (art, edge) {
            (None, Some(edge)) => edge,
            _ => tile.bg,
        };
        r.rect(piece, 3.0, fill);
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        if let Some(uv) = art {
            let uv = [lerp(uv[0], uv[2], part[0]), lerp(uv[1], uv[3], part[1]), lerp(uv[0], uv[2], part[2]), lerp(uv[1], uv[3], part[3])];
            r.image(self.art_atlas, piece, uv, 3.0, Rgba::hex(0xFFFFFF));
            return;
        }
        let Some([u0, v0, u1, v1, aspect]) = self.icons.get(&tile.id).copied() else { return };
        // Where the icon sits in the whole tile (as `draw_tile_icon` places it), cut to the piece.
        let side = rect.h * if edge.is_some() { 0.86 } else { 0.58 };
        let (w, h) = if aspect >= 1.0 { (side * aspect.min(1.8), side) } else { (side, side / aspect) };
        let icon = Rect::new(rect.x + (rect.w - w) * 0.5, rect.y + (rect.h - h) * 0.5, w, h);
        let own = Rect::new(rect.x + rect.w * part[0], rect.y + rect.h * part[1], rect.w * (part[2] - part[0]), rect.h * (part[3] - part[1]));
        let (x0, y0) = (icon.x.max(own.x), icon.y.max(own.y));
        let (x1, y1) = ((icon.x + icon.w).min(own.x + own.w), (icon.y + icon.h).min(own.y + own.h));
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let uv = [
            lerp(u0, u1, (x0 - icon.x) / icon.w),
            lerp(v0, v1, (y0 - icon.y) / icon.h),
            lerp(u0, u1, (x1 - icon.x) / icon.w),
            lerp(v0, v1, (y1 - icon.y) / icon.h),
        ];
        // The piece may be scaled (a tile shrunk before it broke).
        let (sx, sy) = (piece.w / own.w.max(1.0), piece.h / own.h.max(1.0));
        let at = Rect::new(piece.x + (x0 - own.x) * sx, piece.y + (y0 - own.y) * sy, (x1 - x0) * sx, (y1 - y0) * sy);
        r.image(self.icon_atlas, at, uv, 0.0, Rgba::hex(0xFFFFFF));
    }

    /// The sprite is decoded on a worker when the first show starts (the drone appears a few
    /// frames later) and uploaded here once it is ready.
    fn load_visi_sprite(&mut self, r: &Renderer) {
        if self.visi_sprite.is_some() {
            return;
        }
        let Some(decoding) = &self.visi_decoding else {
            let (send, receive) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = send.send(crate::model::decode_png_rgba(SPRITE_PNG));
            });
            self.visi_decoding = Some(receive);
            return;
        };
        let Ok(result) = decoding.try_recv() else { return };
        self.visi_decoding = None;
        self.visi_sprite = Some(match result {
            Ok((pixels, w, h)) => {
                let texture = r.create_texture(w as i32, h as i32, gl::RGBA);
                r.upload(texture, 0, 0, w as i32, h as i32, gl::RGBA, &pixels);
                Some(texture)
            }
            Err(error) => {
                crate::log::log!("visi: {error}");
                None
            }
        });
    }

    /// Starts a show when Home has been left alone long enough (or at once with `--visi`).
    pub(super) fn visi_tick(&mut self, dt: f32) -> bool {
        if let Some(visi) = &mut self.visi {
            if visi.step(dt) {
                return true;
            }
            self.visi = None;
            self.idle_since = Instant::now();
            return true;
        }
        let minutes = self.prefs.ui.visi_minutes;
        let due = self.visi_now
            || (minutes > 0 && !self.prefs.ui.reduced_motion && self.idle_since.elapsed() >= Duration::from_secs(minutes as u64 * 60));
        if !due || !self.visible || !self.loaded || !matches!(self.mode, Mode::Browse) || self.header || self.preview.is_some() {
            return false;
        }
        // Not the Top Shelf's cards: they come and go with the dock's focus.
        let shelf = if self.look == Look::Glass { self.shelf_rails() } else { &[] };
        let targets: Vec<(Rect, usize, usize)> = self
            .hits
            .borrow()
            .iter()
            .filter_map(|&(rect, target)| match target {
                Target::Tile { rail, index } if !shelf.contains(&rail) && rect.y > 0.0 && rect.y + rect.h < LOGICAL_H => Some((rect, rail, index)),
                _ => None,
            })
            .collect();
        let seed = self.visi_seed.unwrap_or_else(|| crate::clock::utc_ms() as u32 | 1);
        // Too few tiles on screen (or not drawn yet): try again later.
        self.visi = Visi::new(targets, self.clock_rect.get(), seed);
        if self.visi.is_none() {
            return false;
        }
        self.visi_now = false;
        self.idle_since = Instant::now();
        true
    }

    /// Any input during a show ends it (and is used up).
    pub(super) fn visi_interrupt(&mut self) -> bool {
        self.idle_since = Instant::now();
        match &mut self.visi {
            Some(visi) => {
                visi.dismiss();
                true
            }
            None => false,
        }
    }
}

/// A hole in the screen: while being cut, a glowing seam along the rim; then black inside,
/// a few stars far away and a red-hot edge.
fn draw_hole(r: &mut Renderer, hole: &Hole, t: f32) {
    let (x, y, radius) = (hole.x, hole.y, hole.r);
    if radius < 1.0 {
        return;
    }
    if hole.cut < 1.0 {
        let steps = (hole.cut * 48.0) as usize;
        for k in 0..steps {
            let a = -PI * 0.5 + k as f32 / 48.0 * TAU;
            let (px, py) = (x + a.cos() * radius, y + a.sin() * radius);
            r.rect(Rect::new(px - 3.0, py - 3.0, 6.0, 6.0), 3.0, Rgba::hex(0xFFB060));
        }
        return;
    }
    let disc = Rect::new(x - radius, y - radius, radius * 2.0, radius * 2.0);
    r.shadow(disc.inflate(10.0), radius + 10.0, 30.0, Rgba::hex(0xFF5A1A).alpha(0.55));
    r.rect(disc, radius, Rgba::hex(0x020306));
    for k in 0..7u32 {
        let a = k as f32 * 2.4 + t * 0.3;
        let d = radius * (0.2 + 0.08 * k as f32).min(0.8);
        let size = 2.0 + (k % 3) as f32;
        r.rect(Rect::new(x + a.cos() * d, y + a.sin() * d, size, size), size * 0.5, Rgba::hex(0xBFD4FF).alpha(0.8));
    }
    r.stroke(disc.inflate(-1.0), radius, 3.0, Rgba::hex(0xFFB060).alpha(0.9));
}

/// The laser graffiti: hot orange where fresh, cooling to a dark burn.
fn draw_ink(r: &mut Renderer, ink: &[(f32, f32, f32, bool)]) {
    for pair in ink.windows(2) {
        let ((x0, y0, _, _), (x1, y1, age, pen)) = (pair[0], pair[1]);
        if !pen {
            continue;
        }
        let hot = (1.0 - age / 1.2).clamp(0.0, 1.0);
        line(r, (x0, y0), (x1, y1), 8.0, Rgba::hex(0x1A0E08).lerp(Rgba::hex(0xFFB060), hot).alpha(0.8));
    }
}
