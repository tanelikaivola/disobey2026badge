//! Vector demo — draws primitives directly to the display surface,
//! no framebuffer. Multiple effects render simultaneously in random
//! combinations, swapping to a new mix every few seconds.

#![no_std]
#![no_main]

use defmt::info;
#[allow(clippy::wildcard_imports)]
use disobey2026badge::*;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{Circle, Line, PrimitiveStyle, Rectangle},
};
use esp_backtrace as _;
use esp_hal::timer::timg::TimerGroup;
use esp_println as _;

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

const W: i32 = 320;
const H: i32 = 170;

// ── Utilities ───────────────────────────────────────────────────────────────

const SIN_Q: [i16; 65] = [
    0, 3, 6, 9, 12, 16, 19, 22, 25, 28, 31, 34, 37, 40, 43, 46, 49, 51, 54, 57, 60, 62, 65, 67,
    70, 72, 75, 77, 79, 81, 84, 86, 88, 90, 92, 93, 95, 97, 99, 100, 102, 103, 105, 106, 107,
    108, 110, 111, 112, 113, 114, 114, 115, 116, 117, 117, 118, 118, 119, 119, 119, 120, 120, 120,
    120,
];

fn isin(angle: i32) -> i32 {
    let a = ((angle % 1024) + 1024) as u32 % 1024;
    let quadrant = a / 256;
    let idx = (a % 256) as usize;
    let i = idx * 64 / 256;
    match quadrant {
        0 => SIN_Q[i] as i32,
        1 => SIN_Q[64 - i] as i32,
        2 => -(SIN_Q[i] as i32),
        _ => -(SIN_Q[64 - i] as i32),
    }
}

fn icos(angle: i32) -> i32 { isin(angle + 256) }

fn hash_u32(mut x: u32) -> u32 {
    x = x.wrapping_mul(2654435761);
    x ^= x >> 16;
    x = x.wrapping_mul(0x45d9f3b);
    x ^= x >> 16;
    x
}

fn hue_color(hue: i32) -> Rgb565 {
    Rgb565::new(
        ((isin(hue * 4) + 120) * 31 / 240) as u8,
        ((icos(hue * 3) + 120) * 63 / 240) as u8,
        ((isin(hue * 6 + 100) + 120) * 31 / 240) as u8,
    )
}

fn draw_line(display: &mut Display, x1: i32, y1: i32, x2: i32, y2: i32, color: Rgb565) {
    Line::new(Point::new(x1, y1), Point::new(x2, y2))
        .into_styled(PrimitiveStyle::with_stroke(color, 1))
        .draw(display)
        .unwrap();
}

fn clear(display: &mut Display) {
    Rectangle::new(Point::zero(), Size::new(W as u32, H as u32))
        .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
        .draw(display)
        .unwrap();
}

// ── Trail ring buffer ────────────────────────────────────────────────────────

const TRAIL_LEN: usize = 12;

struct Trail {
    buf: [(i32, i32, i32, i32); TRAIL_LEN],
    head: usize,
    count: usize,
}

impl Trail {
    const fn new() -> Self {
        Self { buf: [(0, 0, 0, 0); TRAIL_LEN], head: 0, count: 0 }
    }
    fn reset(&mut self) { self.head = 0; self.count = 0; }
    fn push(&mut self, x1: i32, y1: i32, x2: i32, y2: i32) -> Option<(i32, i32, i32, i32)> {
        let erase = if self.count == TRAIL_LEN { Some(self.buf[self.head]) } else { self.count += 1; None };
        self.buf[self.head] = (x1, y1, x2, y2);
        self.head = (self.head + 1) % TRAIL_LEN;
        erase
    }
}

// ── Effect: Spinning fan ────────────────────────────────────────────────────

struct SpinningFan { trail: Trail }

impl SpinningFan {
    const fn new() -> Self { Self { trail: Trail::new() } }
    fn reset(&mut self) { self.trail.reset(); }

    fn tick(&mut self, display: &mut Display, f: u32) {
        let (cx, cy, r) = (W / 2, H / 2, 80i32);
        let angle = f as i32 * 8;
        let x1 = cx + isin(angle) * r / 120;
        let y1 = cy + icos(angle) * r / 120;
        let x2 = cx - isin(angle) * r / 120;
        let y2 = cy - icos(angle) * r / 120;
        if let Some((ox1, oy1, ox2, oy2)) = self.trail.push(x1, y1, x2, y2) {
            draw_line(display, ox1, oy1, ox2, oy2, Rgb565::BLACK);
        }
        draw_line(display, x1, y1, x2, y2, hue_color((f % 128) as i32));
    }
}

// ── Effect: Bouncing lines ──────────────────────────────────────────────────

struct BouncingLines {
    trail: Trail,
    x1: i32, y1: i32, x2: i32, y2: i32,
    dx1: i32, dy1: i32, dx2: i32, dy2: i32,
}

impl BouncingLines {
    const fn new() -> Self {
        Self {
            trail: Trail::new(),
            x1: 10, y1: 10, x2: 300, y2: 150,
            dx1: 3, dy1: 2, dx2: -2, dy2: 3,
        }
    }
    fn reset(&mut self) {
        self.trail.reset();
        self.x1 = 10; self.y1 = 10; self.x2 = 300; self.y2 = 150;
        self.dx1 = 3; self.dy1 = 2; self.dx2 = -2; self.dy2 = 3;
    }

    fn tick(&mut self, display: &mut Display, f: u32) {
        self.x1 += self.dx1; self.y1 += self.dy1;
        self.x2 += self.dx2; self.y2 += self.dy2;
        if self.x1 <= 0 || self.x1 >= W - 1 { self.dx1 = -self.dx1; self.x1 = self.x1.clamp(0, W - 1); }
        if self.y1 <= 0 || self.y1 >= H - 1 { self.dy1 = -self.dy1; self.y1 = self.y1.clamp(0, H - 1); }
        if self.x2 <= 0 || self.x2 >= W - 1 { self.dx2 = -self.dx2; self.x2 = self.x2.clamp(0, W - 1); }
        if self.y2 <= 0 || self.y2 >= H - 1 { self.dy2 = -self.dy2; self.y2 = self.y2.clamp(0, H - 1); }
        if let Some((ox1, oy1, ox2, oy2)) = self.trail.push(self.x1, self.y1, self.x2, self.y2) {
            draw_line(display, ox1, oy1, ox2, oy2, Rgb565::BLACK);
        }
        let hue = (f * 3 % 128) as i32;
        let color = Rgb565::new(
            ((isin(hue * 8) + 120) * 31 / 240) as u8,
            ((icos(hue * 6) + 120) * 63 / 240) as u8,
            ((isin(hue * 10 + 200) + 120) * 31 / 240) as u8,
        );
        draw_line(display, self.x1, self.y1, self.x2, self.y2, color);
    }
}

// ── Effect: Lissajous ───────────────────────────────────────────────────────

struct Lissajous { trail: Trail, prev_x: i32, prev_y: i32 }

impl Lissajous {
    const fn new() -> Self { Self { trail: Trail::new(), prev_x: W / 2, prev_y: H / 2 } }
    fn reset(&mut self) { self.trail.reset(); self.prev_x = W / 2; self.prev_y = H / 2; }

    fn tick(&mut self, display: &mut Display, f: u32) {
        let t = f as i32 * 4;
        let x = W / 2 + isin(t * 3) * 140 / 120;
        let y = H / 2 + isin(t * 2 + 256) * 75 / 120;
        if f > 0 {
            if let Some((ox1, oy1, ox2, oy2)) = self.trail.push(self.prev_x, self.prev_y, x, y) {
                draw_line(display, ox1, oy1, ox2, oy2, Rgb565::BLACK);
            }
            let hue = (f % 256) as i32;
            let color = Rgb565::new(
                ((isin(hue * 4) + 120) * 28 / 240 + 3) as u8,
                ((icos(hue * 3) + 120) * 55 / 240 + 8) as u8,
                ((isin(hue * 5 + 300) + 120) * 28 / 240 + 3) as u8,
            );
            draw_line(display, self.prev_x, self.prev_y, x, y, color);
        }
        self.prev_x = x;
        self.prev_y = y;
    }
}

// ── Effect: Expanding rings ─────────────────────────────────────────────────

const MAX_RINGS: usize = 6;

struct RingState { radius: i32, color: Rgb565, active: bool }

struct Rings {
    rings: [RingState; MAX_RINGS],
    spawn_timer: u32,
}

impl Rings {
    const fn new() -> Self {
        Self {
            rings: [const { RingState { radius: 0, color: Rgb565::BLACK, active: false } }; MAX_RINGS],
            spawn_timer: 0,
        }
    }
    fn reset(&mut self) {
        for r in self.rings.iter_mut() { r.active = false; }
        self.spawn_timer = 0;
    }

    fn tick(&mut self, display: &mut Display, f: u32) {
        let (cx, cy, max_r) = (W / 2, H / 2, 90i32);
        self.spawn_timer += 1;
        if self.spawn_timer >= 18 {
            self.spawn_timer = 0;
            for ring in self.rings.iter_mut() {
                if !ring.active {
                    ring.radius = 4;
                    ring.color = hue_color((f * 5 % 256) as i32);
                    ring.active = true;
                    break;
                }
            }
        }
        for ring in self.rings.iter_mut() {
            if !ring.active { continue; }
            let old_r = ring.radius as u32;
            if old_r > 0 {
                Circle::new(Point::new(cx - old_r as i32, cy - old_r as i32), old_r * 2)
                    .into_styled(PrimitiveStyle::with_stroke(Rgb565::BLACK, 2))
                    .draw(display).unwrap();
            }
            ring.radius += 2;
            if ring.radius > max_r { ring.active = false; continue; }
            let r = ring.radius as u32;
            Circle::new(Point::new(cx - r as i32, cy - r as i32), r * 2)
                .into_styled(PrimitiveStyle::with_stroke(ring.color, 2))
                .draw(display).unwrap();
        }
    }
}

// ── Effect: Raster bars ─────────────────────────────────────────────────────

const NUM_BARS: usize = 5;
const BAR_H: i32 = 14;

struct BarState { y: i32, prev_y: i32, phase: i32, speed: i32, r: u8, g: u8, b: u8 }

struct RasterBars { bars: [BarState; NUM_BARS] }

impl RasterBars {
    const fn new() -> Self {
        Self { bars: [
            BarState { y: 0, prev_y: -BAR_H, phase: 0, speed: 5, r: 31, g: 10, b: 0 },
            BarState { y: 0, prev_y: -BAR_H, phase: 200, speed: 7, r: 0, g: 50, b: 20 },
            BarState { y: 0, prev_y: -BAR_H, phase: 400, speed: 4, r: 20, g: 0, b: 31 },
            BarState { y: 0, prev_y: -BAR_H, phase: 600, speed: 6, r: 31, g: 50, b: 0 },
            BarState { y: 0, prev_y: -BAR_H, phase: 800, speed: 3, r: 0, g: 30, b: 31 },
        ]}
    }
    fn reset(&mut self) {
        for b in self.bars.iter_mut() { b.y = 0; b.prev_y = -BAR_H; }
    }

    fn tick(&mut self, display: &mut Display, f: u32) {
        for bar in self.bars.iter_mut() {
            if bar.prev_y >= 0 && bar.prev_y < H {
                let ey = bar.prev_y.max(0);
                let eh = BAR_H.min(H - ey);
                if eh > 0 {
                    Rectangle::new(Point::new(0, ey), Size::new(W as u32, eh as u32))
                        .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
                        .draw(display).unwrap();
                }
            }
            bar.prev_y = bar.y;
            let wave = isin(f as i32 * bar.speed + bar.phase);
            bar.y = H / 2 + wave * (H / 2 - BAR_H) / 120;
            for row in 0..BAR_H {
                let dy = bar.y + row;
                if dy < 0 || dy >= H { continue; }
                let dist = (row - BAR_H / 2).abs();
                let fade = (BAR_H / 2 - dist).max(0) * 2;
                let r = ((bar.r as i32 * fade / BAR_H).min(31)) as u8;
                let g = ((bar.g as i32 * fade / BAR_H).min(63)) as u8;
                let b = ((bar.b as i32 * fade / BAR_H).min(31)) as u8;
                Rectangle::new(Point::new(0, dy), Size::new(W as u32, 1))
                    .into_styled(PrimitiveStyle::with_fill(Rgb565::new(r, g, b)))
                    .draw(display).unwrap();
            }
        }
    }
}

// ── Effect: Starburst ────────────────────────────────────────────────────────

struct Starburst { cycle: u32 }

impl Starburst {
    const fn new() -> Self { Self { cycle: 60 } }
    fn reset(&mut self) {}

    fn tick(&mut self, display: &mut Display, f: u32) {
        let (cx, cy) = (W / 2, H / 2);
        const NUM_RAYS: i32 = 16;
        let max_len = 100i32;
        let t = (f % self.cycle) as i32;
        let prev_t = if t > 0 { t - 1 } else { self.cycle as i32 - 1 };
        let len = t * max_len / self.cycle as i32;
        let prev_len = prev_t * max_len / self.cycle as i32;

        if t == 0 && f > 0 {
            for i in 0..NUM_RAYS {
                let angle = i * 1024 / NUM_RAYS;
                let ex = cx + isin(angle) * max_len / 120;
                let ey = cy + icos(angle) * max_len / 120;
                draw_line(display, cx, cy, ex, ey, Rgb565::BLACK);
            }
        }
        for i in 0..NUM_RAYS {
            let angle = i * 1024 / NUM_RAYS;
            let dx = isin(angle);
            let dy = icos(angle);
            if t > 0 {
                let px = cx + dx * prev_len / 120;
                let py = cy + dy * prev_len / 120;
                draw_line(display, cx, cy, px, py, Rgb565::BLACK);
            }
            let nx = cx + dx * len / 120;
            let ny = cy + dy * len / 120;
            let hue = ((f * 2 + i as u32 * 8) % 256) as i32;
            draw_line(display, cx, cy, nx, ny, hue_color(hue));
        }
    }
}

// ── Effect: Starfield ───────────────────────────────────────────────────────

const NUM_STARS: usize = 80;
const MAX_Z: i32 = 512;

struct Star3D { x: i32, y: i32, z: i32 }

struct Starfield { stars: [Star3D; NUM_STARS] }

impl Starfield {
    const fn new() -> Self {
        Self { stars: [const { Star3D { x: 0, y: 0, z: 0 } }; NUM_STARS] }
    }

    fn reset(&mut self) {
        for i in 0..NUM_STARS {
            let h = hash_u32(i as u32 * 7919 + 42);
            self.stars[i].x = (h % 600) as i32 - 300;
            self.stars[i].y = ((h >> 10) % 340) as i32 - 170;
            self.stars[i].z = ((h >> 20) % MAX_Z as u32) as i32 + 1;
        }
    }

    fn project(s: &Star3D) -> Option<(i32, i32, i32)> {
        if s.z <= 0 { return None; }
        let sx = W / 2 + s.x * 128 / s.z;
        let sy = H / 2 + s.y * 128 / s.z;
        if sx >= 0 && sx < W && sy >= 0 && sy < H { Some((sx, sy, s.z)) } else { None }
    }

    fn tick(&mut self, display: &mut Display, f: u32) {
        for i in 0..NUM_STARS {
            // Erase old
            if let Some((sx, sy, _)) = Self::project(&self.stars[i]) {
                Rectangle::new(Point::new(sx, sy), Size::new(2, 2))
                    .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
                    .draw(display).unwrap();
            }
            self.stars[i].z -= 4;
            if self.stars[i].z <= 0 || Self::project(&self.stars[i]).is_none() {
                let h = hash_u32(f.wrapping_mul(31).wrapping_add(i as u32 * 997));
                self.stars[i].x = (h % 600) as i32 - 300;
                self.stars[i].y = ((h >> 10) % 340) as i32 - 170;
                self.stars[i].z = MAX_Z;
            }
            if let Some((sx, sy, z)) = Self::project(&self.stars[i]) {
                let brightness = ((MAX_Z - z) * 31 / MAX_Z).clamp(4, 31) as u8;
                let size = if z < MAX_Z / 3 { 2u32 } else { 1 };
                Rectangle::new(Point::new(sx, sy), Size::new(size, size))
                    .into_styled(PrimitiveStyle::with_fill(Rgb565::new(brightness, brightness * 2, brightness)))
                    .draw(display).unwrap();
            }
        }
    }
}

// ── Effect: Wireframe cube ───────────────────────────────────────────────────

const CUBE_VERTS: [[i32; 3]; 8] = [
    [-1, -1, -1], [ 1, -1, -1], [ 1,  1, -1], [-1,  1, -1],
    [-1, -1,  1], [ 1, -1,  1], [ 1,  1,  1], [-1,  1,  1],
];
const CUBE_EDGES: [[usize; 2]; 12] = [
    [0,1],[1,2],[2,3],[3,0],
    [4,5],[5,6],[6,7],[7,4],
    [0,4],[1,5],[2,6],[3,7],
];

struct WireCube {
    prev: [(i32, i32, i32, i32); 12],
    has_prev: bool,
}

impl WireCube {
    const fn new() -> Self {
        Self { prev: [(0, 0, 0, 0); 12], has_prev: false }
    }
    fn reset(&mut self) { self.has_prev = false; }

    fn project_vert(v: [i32; 3], ax: i32, ay: i32, scale: i32) -> (i32, i32) {
        let (mut x, mut y, mut z) = (v[0] * scale, v[1] * scale, v[2] * scale);
        // Rotate around X
        let (ny, nz) = ((y * icos(ax) - z * isin(ax)) / 120, (y * isin(ax) + z * icos(ax)) / 120);
        y = ny; z = nz;
        // Rotate around Y
        let (nx, nz2) = ((x * icos(ay) + z * isin(ay)) / 120, (-x * isin(ay) + z * icos(ay)) / 120);
        x = nx; let _ = nz2;
        let d = (nz2 + 400).max(50);
        (W / 2 + x * 200 / d, H / 2 + y * 200 / d)
    }

    fn tick(&mut self, display: &mut Display, f: u32) {
        let fi = f as i32;
        let (ax, ay, scale) = (fi * 3, fi * 5, 60);

        // Erase previous frame's edges
        if self.has_prev {
            for &(x1, y1, x2, y2) in &self.prev {
                draw_line(display, x1, y1, x2, y2, Rgb565::BLACK);
            }
        }

        // Project and draw new edges
        for (idx, &[a, b]) in CUBE_EDGES.iter().enumerate() {
            let (x1, y1) = Self::project_vert(CUBE_VERTS[a], ax, ay, scale);
            let (x2, y2) = Self::project_vert(CUBE_VERTS[b], ax, ay, scale);
            self.prev[idx] = (x1, y1, x2, y2);
            let color = hue_color((f as i32 + idx as i32 * 20) % 256);
            draw_line(display, x1, y1, x2, y2, color);
        }
        self.has_prev = true;
    }
}

// ── Effect: Sine wave oscilloscope ──────────────────────────────────────────
// Draws a sine wave across the screen, erasing the previous wave each frame.

const SCOPE_POINTS: usize = 64;

struct SineScope {
    prev_y: [i32; SCOPE_POINTS],
    has_prev: bool,
}

impl SineScope {
    const fn new() -> Self {
        Self { prev_y: [0; SCOPE_POINTS], has_prev: false }
    }
    fn reset(&mut self) { self.has_prev = false; }

    fn tick(&mut self, display: &mut Display, f: u32) {
        let fi = f as i32;
        let step = W / SCOPE_POINTS as i32;

        // Erase previous wave
        if self.has_prev {
            for i in 1..SCOPE_POINTS {
                let x1 = (i as i32 - 1) * step;
                let x2 = i as i32 * step;
                draw_line(display, x1, self.prev_y[i - 1], x2, self.prev_y[i], Rgb565::BLACK);
            }
        }

        // Compute and draw new wave (two frequencies mixed)
        let mut cur_y = [0i32; SCOPE_POINTS];
        for i in 0..SCOPE_POINTS {
            let x = i as i32 * step;
            let w1 = isin(x * 6 + fi * 8) * 50 / 120;
            let w2 = isin(x * 14 - fi * 12) * 20 / 120;
            cur_y[i] = H / 2 + w1 + w2;
        }

        for i in 1..SCOPE_POINTS {
            let x1 = (i as i32 - 1) * step;
            let x2 = i as i32 * step;
            let color = hue_color((fi + i as i32 * 4) % 256);
            draw_line(display, x1, cur_y[i - 1], x2, cur_y[i], color);
        }

        self.prev_y = cur_y;
        self.has_prev = true;
    }
}

// ── Effect: Bouncing balls ──────────────────────────────────────────────────

const NUM_BALLS: usize = 6;
const BALL_R: i32 = 8;

struct Ball { x: i32, y: i32, dx: i32, dy: i32 }

struct BouncingBalls {
    balls: [Ball; NUM_BALLS],
}

impl BouncingBalls {
    const fn new() -> Self {
        Self { balls: [const { Ball { x: 0, y: 0, dx: 0, dy: 0 } }; NUM_BALLS] }
    }

    fn reset(&mut self) {
        for i in 0..NUM_BALLS {
            let h = hash_u32(i as u32 * 3571 + 99);
            self.balls[i].x = (h % (W as u32 - BALL_R as u32 * 2)) as i32 + BALL_R;
            self.balls[i].y = ((h >> 8) % (H as u32 - BALL_R as u32 * 2)) as i32 + BALL_R;
            self.balls[i].dx = ((h >> 16) % 5) as i32 - 2;
            self.balls[i].dy = ((h >> 20) % 5) as i32 - 2;
            if self.balls[i].dx == 0 { self.balls[i].dx = 2; }
            if self.balls[i].dy == 0 { self.balls[i].dy = 2; }
        }
    }

    fn tick(&mut self, display: &mut Display, f: u32) {
        for (i, ball) in self.balls.iter_mut().enumerate() {
            // Erase old position
            Circle::new(Point::new(ball.x - BALL_R, ball.y - BALL_R), BALL_R as u32 * 2)
                .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
                .draw(display).unwrap();

            // Move
            ball.x += ball.dx;
            ball.y += ball.dy;
            if ball.x <= BALL_R || ball.x >= W - BALL_R { ball.dx = -ball.dx; ball.x = ball.x.clamp(BALL_R, W - BALL_R); }
            if ball.y <= BALL_R || ball.y >= H - BALL_R { ball.dy = -ball.dy; ball.y = ball.y.clamp(BALL_R, H - BALL_R); }

            // Draw new position
            let color = hue_color((f as i32 * 2 + i as i32 * 40) % 256);
            Circle::new(Point::new(ball.x - BALL_R, ball.y - BALL_R), BALL_R as u32 * 2)
                .into_styled(PrimitiveStyle::with_fill(color))
                .draw(display).unwrap();
        }
    }
}

// ── Effect: Spiral drawer ───────────────────────────────────────────────────
// A point traces an expanding/contracting spiral, leaving a colored trail
// that gets erased by a trailing black point.

struct Spiral {
    trail: Trail,
    prev_x: i32,
    prev_y: i32,
}

impl Spiral {
    const fn new() -> Self {
        Self { trail: Trail::new(), prev_x: W / 2, prev_y: H / 2 }
    }
    fn reset(&mut self) { self.trail.reset(); self.prev_x = W / 2; self.prev_y = H / 2; }

    fn tick(&mut self, display: &mut Display, f: u32) {
        let fi = f as i32;
        // Radius oscillates so the spiral breathes in and out
        let r = 20 + (isin(fi * 2) + 120) * 60 / 240;
        let angle = fi * 12;
        let x = W / 2 + isin(angle) * r / 120;
        let y = H / 2 + icos(angle) * r / 120;

        if f > 0 {
            if let Some((ox1, oy1, ox2, oy2)) = self.trail.push(self.prev_x, self.prev_y, x, y) {
                draw_line(display, ox1, oy1, ox2, oy2, Rgb565::BLACK);
            }
            let color = hue_color((fi * 3) % 256);
            draw_line(display, self.prev_x, self.prev_y, x, y, color);
        }
        self.prev_x = x;
        self.prev_y = y;
    }
}

// ── Effect dispatcher ────────────────────────────────────────────────────────
// Each effect gets an ID. We pick 2-3 random ones to run simultaneously.

const NUM_EFFECTS: usize = 11;
const COMBO_SECS: u64 = 3;

struct AllEffects {
    fan: SpinningFan,
    bounce: BouncingLines,
    lissa: Lissajous,
    rings: Rings,
    bars: RasterBars,
    burst: Starburst,
    stars: Starfield,
    cube: WireCube,
    scope: SineScope,
    balls: BouncingBalls,
    spiral: Spiral,
}

impl AllEffects {
    const fn new() -> Self {
        Self {
            fan: SpinningFan::new(),
            bounce: BouncingLines::new(),
            lissa: Lissajous::new(),
            rings: Rings::new(),
            bars: RasterBars::new(),
            burst: Starburst::new(),
            stars: Starfield::new(),
            cube: WireCube::new(),
            scope: SineScope::new(),
            balls: BouncingBalls::new(),
            spiral: Spiral::new(),
        }
    }

    fn reset(&mut self, id: usize) {
        match id {
            0 => self.fan.reset(),
            1 => self.bounce.reset(),
            2 => self.lissa.reset(),
            3 => self.rings.reset(),
            4 => self.bars.reset(),
            5 => self.burst.reset(),
            6 => self.stars.reset(),
            7 => self.cube.reset(),
            8 => self.scope.reset(),
            9 => self.balls.reset(),
            10 => self.spiral.reset(),
            _ => {}
        }
    }

    fn tick(&mut self, display: &mut Display, id: usize, f: u32) {
        match id {
            0 => self.fan.tick(display, f),
            1 => self.bounce.tick(display, f),
            2 => self.lissa.tick(display, f),
            3 => self.rings.tick(display, f),
            4 => self.bars.tick(display, f),
            5 => self.burst.tick(display, f),
            6 => self.stars.tick(display, f),
            7 => self.cube.tick(display, f),
            8 => self.scope.tick(display, f),
            9 => self.balls.tick(display, f),
            10 => self.spiral.tick(display, f),
            _ => {}
        }
    }
}

const EFFECT_NAMES: [&str; NUM_EFFECTS] = [
    "fan", "bounce", "lissajous", "rings", "bars", "burst", "starfield",
    "cube", "scope", "balls", "spiral",
];

/// Pick 2 or 3 unique effect indices using a deterministic hash of `seed`.
fn pick_combo(seed: u32) -> (usize, [usize; 3]) {
    let h = hash_u32(seed);
    // 2 or 3 effects
    let count = 2 + (h % 2) as usize;

    let a = (h % NUM_EFFECTS as u32) as usize;
    let mut b = ((h / 7 + 3) % NUM_EFFECTS as u32) as usize;
    if b == a { b = (b + 1) % NUM_EFFECTS; }
    let mut c = ((h / 13 + 5) % NUM_EFFECTS as u32) as usize;
    while c == a || c == b { c = (c + 1) % NUM_EFFECTS; }

    (count, [a, b, c])
}

// ── Main ────────────────────────────────────────────────────────────────────

#[embassy_executor::task]
async fn display_task(
    display: &'static mut Display<'static>,
    backlight: &'static mut Backlight,
) {
    info!("Vector demo — random combos, no framebuffer");
    backlight.on();

    let mut effects = AllEffects::new();
    let mut round: u32 = 0;
    let mut global_frame: u32 = 0;

    loop {
        // Pick a new random combination
        let (count, ids) = pick_combo(global_frame.wrapping_add(round.wrapping_mul(12345)));
        round = round.wrapping_add(1);

        // Log what we're running
        match count {
            2 => info!("Combo: {} + {}", EFFECT_NAMES[ids[0]], EFFECT_NAMES[ids[1]]),
            _ => info!("Combo: {} + {} + {}", EFFECT_NAMES[ids[0]], EFFECT_NAMES[ids[1]], EFFECT_NAMES[ids[2]]),
        }

        // Reset chosen effects and clear screen
        clear(display);
        for i in 0..count { effects.reset(ids[i]); }
        // Extra init for starfield (needs position seeding)
        for i in 0..count {
            if ids[i] == 6 { effects.stars.reset(); }
        }

        // Run the combination for COMBO_SECS seconds
        let deadline = embassy_time::Instant::now() + Duration::from_secs(COMBO_SECS);
        let mut f: u32 = 0;
        while embassy_time::Instant::now() < deadline {
            for i in 0..count {
                effects.tick(display, ids[i], f);
            }
            f = f.wrapping_add(1);
            global_frame = global_frame.wrapping_add(1);
            embassy_time::block_for(Duration::from_millis(16));
        }
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let peripherals = disobey2026badge::init();
    let resources = split_resources!(peripherals);

    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let display = mk_static!(Display<'static>, resources.display.into());
    let backlight = mk_static!(Backlight, resources.backlight.into());
    spawner.must_spawn(display_task(display, backlight));

    loop {
        Timer::after(Duration::from_secs(600)).await;
    }
}
