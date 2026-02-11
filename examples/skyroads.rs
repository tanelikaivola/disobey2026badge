//! Skyroads-style pseudo-3D game for the Disobey 2026 badge.
//!
//! Stay on the platforms! Gaps are deadly, blocks destroy you on contact.
//! - Left/Right to steer between lanes
//! - A to jump over gaps and low blocks
//! - Can't jump inside tunnels
//! - LEDs react to speed and state

#![no_std]
#![no_main]

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use defmt::info;
#[allow(clippy::wildcard_imports)]
use disobey2026badge::*;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::*,
    primitives::Rectangle,
};
use esp_backtrace as _;
use esp_hal::timer::timg::TimerGroup;
use esp_println as _;
use palette::Srgb;

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

// Display
const W: i32 = 320;
const H: i32 = 170;
const PIXELS: usize = (W * H) as usize;

// Perspective
const HORIZON_Y: i32 = 50;
const ROAD_H: i32 = H - HORIZON_Y; // 120px
const CAMERA_D: i32 = 60;
const NUM_LANES: i32 = 7;
const ROAD_HW_NEAR: i32 = 150; // half-width at screen bottom

// Ship
const SHIP_W: i32 = 16;
const SHIP_H: i32 = 10;
const SHIP_SCREEN_Y: i32 = H - 26;
/// Ship's world Z derived from its screen position for visual consistency.
const SHIP_Z: i32 = CAMERA_D * ROAD_H / (SHIP_SCREEN_Y - HORIZON_Y);
const JUMP_HEIGHT: i32 = 36;
const JUMP_DURATION: i32 = 26;

// Grid
const GRID_LANES: usize = NUM_LANES as usize;
const GRID_DEPTH: usize = 200;
const CELL_LENGTH: i32 = 12;
/// Max world Z we'll look up in the grid. Beyond this we just draw platform.
const MAX_VIEW_Z: i32 = (GRID_DEPTH as i32 - 4) * CELL_LENGTH;

// Movement: ship X in fixed-point (×256), lane center spacing
const FP: i32 = 256;
const LANE_MOVE_SPEED: i32 = 20; // pixels per tick of lateral movement

const TICK_MS: u64 = 25;

/// Cell types on the grid.
#[derive(Clone, Copy, PartialEq)]
#[repr(u8)]
enum Cell {
    Platform, // safe to drive on
    Gap,      // void — fall to death
    Block,    // low obstacle — jump over or die
    Tunnel,   // ceiling — can't jump, safe to drive through
}

// ── Framebuffer ─────────────────────────────────────────────────────────────

struct Fb {
    buf: &'static mut [Rgb565; PIXELS],
}

impl Fb {
    fn put(&mut self, x: i32, y: i32, color: Rgb565) {
        if x >= 0 && x < W && y >= 0 && y < H {
            self.buf[(y * W + x) as usize] = color;
        }
    }

    fn fill_rect(&mut self, x0: i32, y0: i32, w: i32, h: i32, color: Rgb565) {
        let x1 = x0.max(0);
        let y1 = y0.max(0);
        let x2 = (x0 + w).min(W);
        let y2 = (y0 + h).min(H);
        for y in y1..y2 {
            let off = (y * W) as usize;
            for x in x1..x2 {
                self.buf[off + x as usize] = color;
            }
        }
    }

    fn hline(&mut self, x0: i32, x1: i32, y: i32, color: Rgb565) {
        if y < 0 || y >= H { return; }
        let xa = x0.max(0);
        let xb = x1.min(W);
        let off = (y * W) as usize;
        for x in xa..xb {
            self.buf[off + x as usize] = color;
        }
    }
}

struct SyncBuf(UnsafeCell<[Rgb565; PIXELS]>);
unsafe impl Sync for SyncBuf {}

static FRAMEBUF: SyncBuf = SyncBuf(UnsafeCell::new([Rgb565::BLACK; PIXELS]));
static FRAME_STATE: AtomicU8 = AtomicU8::new(0);

static INPUT_LEFT: AtomicBool = AtomicBool::new(false);
static INPUT_RIGHT: AtomicBool = AtomicBool::new(false);
static INPUT_JUMP: AtomicBool = AtomicBool::new(false);
static INPUT_START: AtomicBool = AtomicBool::new(false);

// ── Simple RNG ──────────────────────────────────────────────────────────────

struct Rng(u32);

impl Rng {
    const fn new(seed: u32) -> Self { Self(seed) }

    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }

    fn range(&mut self, max: u32) -> u32 {
        self.next() % max
    }
}

// ── Perspective ─────────────────────────────────────────────────────────────

/// World Z → screen Y via 1/z perspective.
fn z_to_sy(z: i32) -> i32 {
    if z <= 0 { return H + 100; }
    HORIZON_Y + CAMERA_D * ROAD_H / z
}

/// Screen Y → road half-width (linear in screen space = correct perspective).
fn hw_at_sy(sy: i32) -> i32 {
    let t = sy - HORIZON_Y;
    if t <= 0 { return 0; }
    ROAD_HW_NEAR * t / ROAD_H
}

/// Lane center X on screen at a given screen Y, for lane 0..NUM_LANES-1.
#[allow(dead_code)]
fn lane_center_sx(lane: i32, sy: i32) -> i32 {
    let hw = hw_at_sy(sy);
    let lane_w = hw * 2 / NUM_LANES;
    W / 2 - hw + lane * lane_w + lane_w / 2
}

// ── Game state ──────────────────────────────────────────────────────────────

struct Game {
    grid: [[Cell; GRID_DEPTH]; GRID_LANES],
    grid_offset: u32,
    generated_up_to: u32,
    // Ship position: lane as fixed-point (×FP). lane 0 center = 0, lane 1 = FP, etc.
    ship_lane_fp: i32,
    jump_tick: i32,
    jump_pressed: bool,
    in_tunnel: bool,
    scroll_pos: i32,   // continuous scroll in world units ×256
    speed: i32,        // world units per tick ×256
    score: u32,
    alive: bool,
    fall_timer: i32,
    crash_timer: i32,
    rng: Rng,
    frame: u32,
}

impl Game {
    fn new() -> Self {
        let mid = NUM_LANES / 2;
        let mut g = Self {
            grid: [[Cell::Platform; GRID_DEPTH]; GRID_LANES],
            grid_offset: 0,
            generated_up_to: 0,
            ship_lane_fp: mid * FP,
            jump_tick: 0,
            jump_pressed: false,
            in_tunnel: false,
            scroll_pos: 0,
            speed: 2 * 256,
            score: 0,
            alive: true,
            fall_timer: 0,
            crash_timer: 0,
            rng: Rng::new(0xDEAD_BEEF),
            frame: 0,
        };
        g.generate_up_to(GRID_DEPTH as u32);
        g
    }

    /// Current lane (integer) the ship is closest to.
    fn current_lane(&self) -> i32 {
        ((self.ship_lane_fp + FP / 2) / FP).clamp(0, NUM_LANES - 1)
    }

    fn ship_jump_y(&self) -> i32 {
        if self.jump_tick <= 0 { return 0; }
        let half = JUMP_DURATION / 2;
        let t = self.jump_tick - half;
        JUMP_HEIGHT - JUMP_HEIGHT * t * t / (half * half)
    }

    /// The number of whole cells that have scrolled past the camera.
    fn cells_scrolled(&self) -> u32 {
        (self.scroll_pos / 256 / CELL_LENGTH) as u32
    }

    /// Sub-cell offset in world units (0..CELL_LENGTH-1) for smooth rendering.
    fn sub_cell_offset(&self) -> i32 {
        (self.scroll_pos / 256) % CELL_LENGTH
    }

    fn cell_at(&self, lane: i32, world_z: i32) -> Cell {
        if lane < 0 || lane >= NUM_LANES { return Cell::Gap; }
        let cell = world_z / CELL_LENGTH;
        if cell < 0 { return Cell::Gap; }
        // Don't read beyond what fits in the ring buffer
        if world_z >= MAX_VIEW_Z { return Cell::Platform; }
        let idx = (cell as u32 + self.cells_scrolled()) as usize % GRID_DEPTH;
        self.grid[lane as usize][idx]
    }

    /// Generate level data. Skyroads style: platforms with gaps, blocks,
    /// and tunnels. Difficulty increases over time.
    fn generate_up_to(&mut self, up_to: u32) {
        while self.generated_up_to < up_to {
            let cell = self.generated_up_to;

            if cell < 20 {
                // Safe starting zone
                let idx = (cell as usize) % GRID_DEPTH;
                for lane in 0..GRID_LANES {
                    self.grid[lane][idx] = Cell::Platform;
                }
                self.generated_up_to += 1;
                continue;
            }

            // Difficulty ramps up: more gaps, less recovery
            let difficulty = ((cell - 20) / 60).min(5) as u32; // 0-5
            let section_roll = self.rng.range(100);

            if section_roll < 20 - difficulty * 2 {
                // Short straight (3-6 cells)
                let len = 3 + self.rng.range(4) as u32;
                self.emit_rows(len, |_, lane, _| {
                    let _ = lane;
                    Cell::Platform
                });
            } else if section_roll < 45 {
                // Gap across most lanes — must find the safe path or jump
                let gap_len = 2 + self.rng.range(2 + difficulty) as u32;
                let safe_center = self.rng.range(GRID_LANES as u32) as i32;
                let safe_radius = if difficulty < 2 { 2 } else { 1 };
                self.emit_rows(gap_len, |_, lane, _| {
                    if (lane as i32 - safe_center).abs() <= safe_radius {
                        Cell::Platform
                    } else {
                        Cell::Gap
                    }
                });
                // Short recovery
                let recov = (3 - difficulty / 2).max(1) as u32;
                self.emit_rows(recov, |_, _, _| Cell::Platform);
            } else if section_roll < 60 {
                // Wide gap — jumpable (2 cells), all lanes
                self.emit_rows(2, |_, _, _| Cell::Gap);
                // Landing platform
                self.emit_rows(2, |_, _, _| Cell::Platform);
            } else if section_roll < 75 {
                // Blocks on many lanes — dodge sideways or jump
                let num_blocked = 2 + self.rng.range(3 + difficulty) as usize;
                let mut blocked = [false; GRID_LANES];
                for _ in 0..num_blocked.min(GRID_LANES - 1) {
                    let l = self.rng.range(GRID_LANES as u32) as usize;
                    blocked[l] = true;
                }
                // Ensure at least one lane is clear
                if blocked.iter().all(|&b| b) {
                    blocked[self.rng.range(GRID_LANES as u32) as usize] = false;
                }
                let blen = 1 + self.rng.range(2) as u32;
                self.emit_rows(blen, |_, lane, _| {
                    if blocked[lane] { Cell::Block } else { Cell::Platform }
                });
                let recov = (2 - difficulty / 3).max(1) as u32;
                self.emit_rows(recov, |_, _, _| Cell::Platform);
            } else if section_roll < 88 {
                // Tunnel with gaps outside — forces you into tunnel lanes
                let tunnel_center = 1 + self.rng.range((GRID_LANES - 2) as u32) as i32;
                let tunnel_len = 4 + self.rng.range(4 + difficulty) as u32;
                let rng_seed = self.rng.next();
                let mut local_rng = Rng::new(rng_seed);
                self.emit_rows(tunnel_len, |row, lane, _| {
                    let dist = (lane as i32 - tunnel_center).abs();
                    if dist <= 1 {
                        Cell::Tunnel
                    } else if row > 0 && row < tunnel_len as usize - 1
                        && local_rng.range(3) == 0
                    {
                        Cell::Gap
                    } else {
                        Cell::Platform
                    }
                });
                self.emit_rows(2, |_, _, _| Cell::Platform);
            } else {
                // Narrow bridge — only center lanes, rest is void
                let bridge_center = self.rng.range(GRID_LANES as u32) as i32;
                let bridge_len = 4 + self.rng.range(4 + difficulty) as u32;
                self.emit_rows(bridge_len, |_, lane, _| {
                    if (lane as i32 - bridge_center).abs() <= 1 {
                        Cell::Platform
                    } else {
                        Cell::Gap
                    }
                });
                self.emit_rows(2, |_, _, _| Cell::Platform);
            }
        }
    }

    /// Helper: emit `count` rows using a closure that returns the cell type
    /// for each (row_index, lane, global_cell_index).
    /// Stops early if we'd overwrite cells still in the visible ring buffer.
    fn emit_rows(&mut self, count: u32, mut f: impl FnMut(usize, usize, u32) -> Cell) {
        let max_cell = self.grid_offset + GRID_DEPTH as u32;
        for row in 0..count {
            if self.generated_up_to >= max_cell {
                return;
            }
            let cell = self.generated_up_to;
            let idx = (cell as usize) % GRID_DEPTH;
            for lane in 0..GRID_LANES {
                self.grid[lane][idx] = f(row as usize, lane, cell);
            }
            self.generated_up_to += 1;
        }
    }

    fn tick(&mut self) {
        if !self.alive { return; }

        if self.fall_timer > 0 {
            self.fall_timer += 1;
            if self.fall_timer > 20 { self.alive = false; }
            return;
        }
        if self.crash_timer > 0 {
            self.crash_timer += 1;
            if self.crash_timer > 15 { self.alive = false; }
            return;
        }

        let left = INPUT_LEFT.load(Ordering::Relaxed);
        let right = INPUT_RIGHT.load(Ordering::Relaxed);
        let jump = INPUT_JUMP.load(Ordering::Relaxed);

        // Lateral movement: free continuous movement while button held
        if left {
            self.ship_lane_fp -= LANE_MOVE_SPEED;
        }
        if right {
            self.ship_lane_fp += LANE_MOVE_SPEED;
        }
        self.ship_lane_fp = self.ship_lane_fp.clamp(0, (NUM_LANES - 1) * FP);

        // Check if in tunnel
        let lane = self.current_lane();
        self.in_tunnel = self.cell_at(lane, SHIP_Z) == Cell::Tunnel;

        // Jump (edge-triggered, blocked in tunnel)
        if jump && !self.jump_pressed && self.jump_tick <= 0 && !self.in_tunnel {
            self.jump_tick = JUMP_DURATION;
        }
        self.jump_pressed = jump;
        if self.jump_tick > 0 {
            self.jump_tick -= 1;
        }
        // If we enter a tunnel while jumping, cancel the jump
        if self.in_tunnel && self.jump_tick > 0 {
            self.jump_tick = 0;
        }

        // Scroll forward
        let old_cells = self.cells_scrolled();
        self.scroll_pos += self.speed;
        let new_cells = self.cells_scrolled();
        if new_cells > old_cells {
            self.grid_offset = new_cells;
            let need = self.grid_offset + GRID_DEPTH as u32;
            if need > self.generated_up_to {
                self.generate_up_to(need);
            }
        }

        // Speed up gradually
        if self.frame % 120 == 0 && self.speed < 6 * 256 {
            self.speed += 12;
        }

        // Collision
        let cell = self.cell_at(lane, SHIP_Z);
        let airborne = self.ship_jump_y() > 6;

        match cell {
            Cell::Gap => {
                if !airborne {
                    self.fall_timer = 1;
                }
            }
            Cell::Block => {
                if !airborne {
                    self.crash_timer = 1;
                }
            }
            Cell::Tunnel | Cell::Platform => {}
        }

        self.score += 1;
        self.frame += 1;
    }
}

// ── Rendering ───────────────────────────────────────────────────────────────

fn render_sky(fb: &mut Fb) {
    for y in 0..HORIZON_Y {
        let t = y * 31 / HORIZON_Y.max(1);
        let r = (1 + t / 10) as u8;
        let g = (1 + t / 5) as u8;
        let b = (6 + t / 3) as u8;
        fb.hline(0, W, y, Rgb565::new(r, g, b));
    }
    let mut rng = Rng::new(42);
    for _ in 0..40 {
        let x = rng.range(W as u32) as i32;
        let y = rng.range(HORIZON_Y.max(1) as u32) as i32;
        let bright = 16 + rng.range(16) as u8;
        fb.put(x, y, Rgb565::new(bright, bright * 2, bright));
    }
}

fn render_road(fb: &mut Fb, game: &Game) {
    let cx = W / 2;

    for sy in HORIZON_Y..H {
        let t = sy - HORIZON_Y;
        if t <= 0 { continue; }

        // Raw depth without sub-cell offset — stable per screen row
        let raw_z = CAMERA_D * ROAD_H / t;

        let hw = hw_at_sy(sy);
        let lane_w = hw * 2 / NUM_LANES;
        if lane_w <= 0 { continue; }

        let fog = (31 - t * 31 / ROAD_H).clamp(0, 31) as u8;

        // World-space checker band (stable, shifts in whole-cell steps)
        let band = raw_z / CELL_LENGTH + game.cells_scrolled() as i32;

        for lane_i in 0..NUM_LANES {
            let lx = cx - hw + lane_i * lane_w;
            let rx = lx + lane_w;
            // Use raw_z for cell lookup — no sub-pixel jitter on cell boundaries
            let cell = game.cell_at(lane_i, raw_z);
            let checker = ((band + lane_i as i32) & 1) == 0;

            match cell {
                Cell::Platform => {
                    let (r, g, b) = if checker {
                        (2 + fog / 8, 5 + fog / 2, 7 + fog / 2)
                    } else {
                        (1 + fog / 10, 3 + fog / 3, 5 + fog / 3)
                    };
                    fb.hline(lx + 1, rx, sy, Rgb565::new(r, g, b));
                    fb.put(lx, sy, Rgb565::new(3 + fog / 4, 8 + fog / 2, 5 + fog / 3));
                }
                Cell::Gap => {
                    let void_b = (fog / 8).min(3);
                    fb.hline(lx, rx, sy, Rgb565::new(0, 0, void_b));
                }
                Cell::Block => {
                    // Block: reddish raised surface
                    let (r, g, b) = if checker {
                        (12 + fog / 4, 2 + fog / 8, 2 + fog / 10)
                    } else {
                        (8 + fog / 4, 1 + fog / 10, 1 + fog / 12)
                    };
                    fb.hline(lx + 1, rx, sy, Rgb565::new(r, g, b));
                    fb.put(lx, sy, Rgb565::new(16 + fog / 3, 4, 2));
                }
                Cell::Tunnel => {
                    // Tunnel: platform with ceiling indicator (darker, yellowish)
                    let (r, g, b) = if checker {
                        (4 + fog / 6, 4 + fog / 4, 1 + fog / 8)
                    } else {
                        (3 + fog / 8, 3 + fog / 5, 1 + fog / 10)
                    };
                    fb.hline(lx + 1, rx, sy, Rgb565::new(r, g, b));
                    fb.put(lx, sy, Rgb565::new(6 + fog / 4, 6 + fog / 3, 2));
                }
            }
        }

        // Void outside road
        fb.hline(0, cx - hw, sy, Rgb565::new(0, 0, 1));
        fb.hline(cx + hw, W, sy, Rgb565::new(0, 0, 1));
    }
}

/// Render block and tunnel 3D faces (front faces of blocks/tunnels near camera).
fn render_obstacles_3d(fb: &mut Fb, game: &Game) {
    let sub_offset = game.sub_cell_offset();

    // Iterate cell boundaries. cell_off is the cell index relative to camera.
    // Screen position uses sub_offset for smooth sliding.
    // Cell type lookup uses the raw cell index (no sub_offset) to match the road.
    for cell_off in 0..20i32 {
        // Screen position: smooth with sub_offset
        let screen_z = cell_off * CELL_LENGTH + CELL_LENGTH - sub_offset;
        if screen_z <= 1 { continue; }

        let sy_back = z_to_sy(screen_z);
        let sy_front = z_to_sy(screen_z - CELL_LENGTH);
        if sy_back <= HORIZON_Y { continue; }
        if sy_front <= HORIZON_Y { continue; }

        // Cell lookup: use raw cell index (matches road scanline renderer)
        let lookup_z = cell_off * CELL_LENGTH + CELL_LENGTH;

        let hw_b = hw_at_sy(sy_back);
        let lane_w_b = hw_b * 2 / NUM_LANES;
        let cx = W / 2;

        for lane_i in 0..NUM_LANES {
            let cell = game.cell_at(lane_i, lookup_z - 1);
            let cell_behind = game.cell_at(lane_i, lookup_z);

            if cell == Cell::Block && cell_behind != Cell::Block {
                // Front face of block
                let lx = cx - hw_b + lane_i * lane_w_b;
                let rx = lx + lane_w_b;
                let block_h = ((sy_front - sy_back) * 2 / 3).max(2).min(20);
                let top_y = sy_back - block_h;
                fb.fill_rect(lx + 1, top_y, rx - lx - 1, block_h, Rgb565::new(24, 6, 4));
                fb.hline(lx + 1, rx, top_y, Rgb565::new(31, 12, 8));
            }

            if cell == Cell::Tunnel {
                // Tunnel ceiling: draw a bar across the top
                let lx = cx - hw_b + lane_i * lane_w_b;
                let rx = lx + lane_w_b;
                let ceil_h = ((sy_front - sy_back) / 2).max(1).min(12);
                let top_y = sy_back - ceil_h;
                fb.fill_rect(lx, top_y, rx - lx, ceil_h, Rgb565::new(8, 8, 3));
                fb.hline(lx, rx, top_y, Rgb565::new(12, 12, 4));
            }
        }
    }
}

fn render_ship(fb: &mut Fb, game: &Game) {
    // Use the near road edge (at screen bottom) for ship positioning
    // This keeps the ship visually within the road regardless of CAMERA_D/SHIP_Z
    let hw = ROAD_HW_NEAR;
    let lane_w = hw * 2 / NUM_LANES;
    let cx = W / 2;

    let ship_center_x = cx - hw + game.ship_lane_fp * lane_w / FP + lane_w / 2;
    let ship_x = ship_center_x - SHIP_W / 2;

    let jump_y = game.ship_jump_y();
    let fall_off = if game.fall_timer > 0 {
        game.fall_timer * game.fall_timer / 2
    } else {
        0
    };
    let crash_off = if game.crash_timer > 0 {
        // Shake effect
        ((game.crash_timer * 7) % 5) - 2
    } else {
        0
    };
    let ship_y = SHIP_SCREEN_Y - jump_y + fall_off;
    let ship_x = ship_x + crash_off;

    // Shadow
    if jump_y > 4 && game.fall_timer == 0 {
        let sw = SHIP_W - jump_y / 3;
        let sx = ship_center_x - sw / 2;
        fb.fill_rect(sx, SHIP_SCREEN_Y + 2, sw, 2, Rgb565::new(1, 3, 2));
    }

    // Body
    fb.fill_rect(ship_x + 3, ship_y + 3, SHIP_W - 6, SHIP_H - 3, Rgb565::new(6, 20, 31));
    // Nose
    fb.fill_rect(ship_x + SHIP_W / 2 - 2, ship_y, 4, 4, Rgb565::new(12, 28, 31));
    // Wings
    fb.fill_rect(ship_x, ship_y + 3, 3, SHIP_H - 5, Rgb565::new(4, 14, 24));
    fb.fill_rect(ship_x + SHIP_W - 3, ship_y + 3, 3, SHIP_H - 5, Rgb565::new(4, 14, 24));

    // Engine glow
    if game.fall_timer == 0 && game.crash_timer == 0 {
        let glow = if game.frame % 4 < 2 {
            Rgb565::new(31, 20, 4)
        } else {
            Rgb565::new(31, 10, 0)
        };
        fb.fill_rect(ship_x + 4, ship_y + SHIP_H - 1, 3, 2, glow);
        fb.fill_rect(ship_x + SHIP_W - 7, ship_y + SHIP_H - 1, 3, 2, glow);
    }

    // Tunnel ceiling warning: if in tunnel, draw ceiling bar over ship
    if game.in_tunnel {
        let ceil_y = SHIP_SCREEN_Y - JUMP_HEIGHT + 4;
        fb.hline(ship_x - 2, ship_x + SHIP_W + 2, ceil_y, Rgb565::new(12, 12, 4));
        fb.hline(ship_x - 2, ship_x + SHIP_W + 2, ceil_y + 1, Rgb565::new(8, 8, 3));
    }
}

fn render_hud(fb: &mut Fb, score: u32, speed: i32) {
    let speed_norm = ((speed / 256 - 2) * 60 / 4).clamp(0, 60);
    fb.fill_rect(4, 4, 62, 6, Rgb565::new(2, 4, 2));
    fb.fill_rect(5, 5, speed_norm, 4, Rgb565::new(4, 20, 4));

    let mut buf = [0u8; 16];
    let s = format_u32(score, &mut buf);
    let sx = W - 6 * s.len() as i32 - 4;
    for (i, ch) in s.bytes().enumerate() {
        let digit = ch - b'0';
        let dx = sx + i as i32 * 6;
        fb.fill_rect(dx, 4, 5, 7, Rgb565::new(1, 2, 4));
        let bright = 10 + digit as u8 * 2;
        fb.fill_rect(dx + 1, 5, 3, 5, Rgb565::new(bright, bright * 2, bright));
    }
}

fn render_frame(fb: &mut Fb, game: &Game) {
    fb.buf.fill(Rgb565::BLACK);
    render_sky(fb);
    render_road(fb, game);
    render_obstacles_3d(fb, game);
    render_ship(fb, game);
    render_hud(fb, game.score, game.speed);
}

// ── Tasks ───────────────────────────────────────────────────────────────────

#[embassy_executor::task]
async fn input_task(buttons: &'static mut Buttons) {
    info!("Input task started");
    loop {
        INPUT_LEFT.store(buttons.left.is_low(), Ordering::Relaxed);
        INPUT_RIGHT.store(buttons.right.is_low(), Ordering::Relaxed);
        INPUT_JUMP.store(buttons.a.is_low(), Ordering::Relaxed);
        INPUT_START.store(buttons.start.is_low(), Ordering::Relaxed);
        Timer::after(Duration::from_millis(10)).await;
    }
}

#[embassy_executor::task]
async fn display_blit_task(display: &'static mut Display<'static>) {
    info!("Display blit task running on core 1");
    loop {
        if FRAME_STATE.load(Ordering::Acquire) == 1 {
            FRAME_STATE.store(2, Ordering::Release);
            let src: &[Rgb565; PIXELS] = unsafe { &*FRAMEBUF.0.get() };
            let area = Rectangle::new(Point::zero(), Size::new(W as u32, H as u32));
            display.fill_contiguous(&area, src.iter().copied()).unwrap();
            FRAME_STATE.store(0, Ordering::Release);
        } else {
            Timer::after(Duration::from_millis(1)).await;
        }
    }
}

fn format_u32(mut n: u32, buf: &mut [u8; 16]) -> &str {
    if n == 0 {
        buf[0] = b'0';
        return unsafe { core::str::from_utf8_unchecked(&buf[..1]) };
    }
    let mut i = 0;
    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    buf[..i].reverse();
    unsafe { core::str::from_utf8_unchecked(&buf[..i]) }
}

#[embassy_executor::task]
async fn game_task(leds: &'static mut Leds<'static>) {
    info!("Skyroads game task started");

    loop {
        // ── Title screen ────────────────────────────────────────────────
        {
            while FRAME_STATE.load(Ordering::Acquire) != 0 {
                Timer::after(Duration::from_millis(1)).await;
            }
            let fb_buf: &'static mut [Rgb565; PIXELS] = unsafe { &mut *FRAMEBUF.0.get() };
            let fb = &mut Fb { buf: fb_buf };
            fb.buf.fill(Rgb565::BLACK);
            render_sky(fb);

            fb.fill_rect(60, 40, 200, 50, Rgb565::new(1, 3, 6));
            fb.fill_rect(62, 42, 196, 46, Rgb565::new(0, 1, 3));

            let title = [
                // S
                (70, 50, 4, 2), (70, 52, 2, 4), (70, 56, 4, 2), (72, 58, 2, 4), (70, 62, 4, 2),
                // K
                (78, 50, 2, 14), (80, 56, 2, 2), (82, 54, 2, 2), (84, 52, 2, 2),
                (82, 58, 2, 2), (84, 60, 2, 2),
                // Y
                (90, 50, 2, 4), (94, 50, 2, 4), (92, 54, 2, 10),
                // R
                (100, 50, 2, 14), (102, 50, 4, 2), (104, 52, 2, 4), (102, 56, 4, 2),
                (104, 58, 2, 2), (104, 60, 2, 4),
                // O
                (110, 50, 6, 2), (110, 62, 6, 2), (110, 52, 2, 10), (114, 52, 2, 10),
                // A
                (120, 52, 2, 12), (126, 52, 2, 12), (122, 50, 4, 2), (122, 58, 4, 2),
                // D
                (130, 50, 2, 14), (132, 50, 4, 2), (136, 52, 2, 10), (132, 62, 4, 2),
                // S
                (142, 50, 4, 2), (142, 52, 2, 4), (142, 56, 4, 2), (144, 58, 2, 4),
                (142, 62, 4, 2),
            ];
            for &(x, y, w, h) in &title {
                fb.fill_rect(x, y, w, h, Rgb565::new(8, 24, 31));
            }

            for i in 0..5 {
                fb.fill_rect(145 + i * 6, 102, 3, 6, Rgb565::new(10, 20, 10));
            }

            FRAME_STATE.store(1, Ordering::Release);
        }

        let mut t: u8 = 0;
        loop {
            if INPUT_START.load(Ordering::Relaxed) || INPUT_JUMP.load(Ordering::Relaxed) {
                break;
            }
            let bright = if t < 32 { t } else { 64 - t };
            leds.fill(Srgb::new(0, bright / 2, bright));
            leds.update().await;
            t = (t + 1) % 64;
            Timer::after(Duration::from_millis(30)).await;
        }

        leds.clear();
        leds.update().await;
        Timer::after(Duration::from_millis(200)).await;

        // ── Game loop ───────────────────────────────────────────────────
        let mut game = Game::new();
        let tick = Duration::from_millis(TICK_MS);

        while game.alive {
            game.tick();

            while FRAME_STATE.load(Ordering::Acquire) != 0 {
                Timer::after(Duration::from_millis(1)).await;
            }
            let fb_buf: &'static mut [Rgb565; PIXELS] = unsafe { &mut *FRAMEBUF.0.get() };
            let fb = &mut Fb { buf: fb_buf };
            render_frame(fb, &game);
            FRAME_STATE.store(1, Ordering::Release);

            // LEDs
            let speed_frac = ((game.speed / 256 - 2) * 5 / 4).clamp(0, 4) as usize;
            let mut bar = [Srgb::new(0u8, 0, 0); BAR_COUNT];
            for i in 0..=speed_frac {
                bar[i] = Srgb::new(0, (5 + i * 4) as u8, (10 - i * 2) as u8);
            }
            if game.jump_tick > 0 {
                bar[4] = Srgb::new(0, 0, 20);
            }
            if game.in_tunnel {
                bar[0] = Srgb::new(10, 10, 2);
            }
            if game.fall_timer > 0 || game.crash_timer > 0 {
                leds.fill(Srgb::new(20, 0, 0));
            } else {
                leds.set_both_bars(&bar);
            }
            leds.update().await;

            Timer::after(tick).await;
        }

        // ── Death ───────────────────────────────────────────────────────
        for flash in 0..6 {
            if flash % 2 == 0 {
                leds.fill(Srgb::new(25, 0, 0));
            } else {
                leds.clear();
            }
            leds.update().await;
            Timer::after(Duration::from_millis(150)).await;
        }

        {
            while FRAME_STATE.load(Ordering::Acquire) != 0 {
                Timer::after(Duration::from_millis(1)).await;
            }
            let fb_buf: &'static mut [Rgb565; PIXELS] = unsafe { &mut *FRAMEBUF.0.get() };
            let fb = &mut Fb { buf: fb_buf };
            fb.buf.fill(Rgb565::new(2, 0, 0));
            fb.fill_rect(80, 50, 160, 30, Rgb565::new(8, 0, 0));
            fb.fill_rect(82, 52, 156, 26, Rgb565::new(4, 0, 0));

            let mut buf = [0u8; 16];
            let s = format_u32(game.score, &mut buf);
            let sx = 160 - 3 * s.len() as i32;
            for (i, ch) in s.bytes().enumerate() {
                let digit = ch - b'0';
                let dx = sx + i as i32 * 6;
                let bright = 10 + digit as u8 * 2;
                fb.fill_rect(dx, 95, 5, 7, Rgb565::new(1, 2, 4));
                fb.fill_rect(dx + 1, 96, 3, 5, Rgb565::new(bright, bright * 2, bright));
            }

            FRAME_STATE.store(1, Ordering::Release);
        }

        leds.clear();
        leds.update().await;

        Timer::after(Duration::from_millis(500)).await;
        loop {
            if INPUT_START.load(Ordering::Relaxed) || INPUT_JUMP.load(Ordering::Relaxed) {
                break;
            }
            Timer::after(Duration::from_millis(50)).await;
        }
        Timer::after(Duration::from_millis(200)).await;
    }
}

// ── Entry point ─────────────────────────────────────────────────────────────

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let peripherals = disobey2026badge::init();
    let resources = split_resources!(peripherals);

    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let buttons = mk_static!(Buttons, resources.buttons.into());
    let leds = mk_static!(Leds<'static>, resources.leds.into());

    use esp_hal::interrupt::software::SoftwareInterruptControl;
    let sw_ints = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);

    let core1_stack = mk_static!(
        esp_hal::system::Stack<8192>,
        esp_hal::system::Stack::new()
    );

    esp_rtos::start_second_core::<8192>(
        peripherals.CPU_CTRL,
        sw_ints.software_interrupt0,
        sw_ints.software_interrupt1,
        core1_stack,
        || {
            let executor = mk_static!(
                esp_rtos::embassy::Executor,
                esp_rtos::embassy::Executor::new()
            );
            executor.run(|spawner| {
                let display = mk_static!(Display<'static>, resources.display.into());
                let backlight = mk_static!(Backlight, resources.backlight.into());
                backlight.on();
                spawner.must_spawn(display_blit_task(display));
            });
        },
    );

    spawner.must_spawn(input_task(buttons));
    spawner.must_spawn(game_task(leds));

    loop {
        Timer::after(Duration::from_secs(600)).await;
    }
}
