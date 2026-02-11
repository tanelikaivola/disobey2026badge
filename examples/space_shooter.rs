//! Side-scrolling space shooter using ST7789 hardware vertical scrolling.
//!
//! The ST7789's VSCRDEF/VSCRSADD commands scroll the background starfield
//! horizontally (the native 320px axis becomes X with Deg90 rotation).
//! Fixed scroll regions create HUD strips on the left and right screen edges.
//!
//! Only the newly-revealed background column and dirty sprite regions are
//! redrawn each frame — no full framebuffer needed.
//!
//! Controls:
//! - D-pad Up/Down: move ship
//! - A: fire
//! - Start: restart after game over

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicBool, Ordering};

use defmt::info;
#[allow(clippy::wildcard_imports)]
use disobey2026badge::*;
use embassy_executor::Spawner;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Instant, Timer};
use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::{FONT_4X6, FONT_6X10}, iso_8859_1::FONT_10X20},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::Text,
};
use esp_backtrace as _;
use esp_hal::timer::timg::TimerGroup;
use esp_println as _;
use palette::Srgb;

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

// ── Display geometry ────────────────────────────────────────────────────────
const SCREEN_W: i32 = 320;
const SCREEN_H: i32 = 170;

// HUD strips in the scroll axis (native rows).
const HUD_RIGHT: u16 = 24;
const HUD_LEFT: u16 = 24;
const SCROLL_AREA: u16 = 320 - HUD_RIGHT - HUD_LEFT;

const GAME_X: i32 = HUD_LEFT as i32;
const GAME_W: i32 = SCROLL_AREA as i32;
const GAME_H: i32 = SCREEN_H;

// ── Tuning ──────────────────────────────────────────────────────────────────
const TICK_MS: u64 = 16;
const SCROLL_SPEED: u16 = 1;
const PLAYER_SPEED: i32 = 2;
const BULLET_SPEED: i32 = 3;
const ENEMY_SPEED: i32 = 1;
const MAX_BULLETS: usize = 12;
const MAX_ENEMIES: usize = 8;
const ENEMY_HP: u8 = 3;
const FIRE_COOLDOWN: u8 = 12;

// ── Input atomics ───────────────────────────────────────────────────────────
static INPUT_UP: AtomicBool = AtomicBool::new(false);
static INPUT_DOWN: AtomicBool = AtomicBool::new(false);
static INPUT_FIRE: AtomicBool = AtomicBool::new(false);
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
    fn range(&mut self, max: u32) -> u32 { self.next() % max }
}

// ── Sine table for fire shader (fixed-point, 0..1023 → -120..120) ──────────
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

/// Space nebula shader — slowly cycles through nebula hues as world_x advances.
fn fire_bg(x: i32, y: i32, frame: i32) -> Rgb565 {
    let fy = GAME_H as i32 - 1 - y;
    let n1 = isin(x * 7 + fy * 3 - frame * 8);
    let n2 = icos(x * 3 + fy * 9 - frame * 12);
    let n3 = isin((x + fy) * 5 - frame * 6);
    let heat = (n1 + n2 + n3 + 360) * fy / (GAME_H as i32 * 2);
    let heat = heat.clamp(0, 180) as u32;

    // Contrast curve: h² / 180 maps 0..180 → 0..180 with darker darks
    let h = (heat * heat / 180) as u32;

    // Slowly rotating hue based on world x position (full cycle ~2048 px)
    let phase = x / 3 + 512; // start in blue range
    let wr = (isin(phase) + 120) as u32;       // 0..240
    let wg = (isin(phase + 341) + 120) as u32; // 120° offset
    let wb = (isin(phase + 682) + 120) as u32; // 240° offset

    let r = (h * wr / (240 * 2)).min(31) as u8;
    let g = (h * wg / (240 * 1)).min(63) as u8;
    let b = (h * wb / (240 * 1)).min(31) as u8;
    Rgb565::new(r, g, b)
}

// ── Weapon configurations ───────────────────────────────────────────────────
#[derive(Clone, Copy)]
struct WeaponConfig {
    count: u8,
    offsets: [i32; 4],
    color: Rgb565,
    damage: u8,
    name: &'static [u8],
}

const WEAPON_SINGLE: WeaponConfig = WeaponConfig {
    count: 1, offsets: [0, 0, 0, 0],
    color: Rgb565::YELLOW, damage: 1, name: b"SNGL",
};
const WEAPON_DOUBLE: WeaponConfig = WeaponConfig {
    count: 2, offsets: [-4, 4, 0, 0],
    color: Rgb565::CYAN, damage: 1, name: b"DUAL",
};
const WEAPON_SPREAD: WeaponConfig = WeaponConfig {
    count: 3, offsets: [-6, 0, 6, 0],
    color: Rgb565::CSS_ORANGE, damage: 1, name: b"SPRD",
};
const WEAPONS: &[WeaponConfig] = &[WEAPON_SINGLE, WEAPON_DOUBLE, WEAPON_SPREAD];

// ── Entity types ────────────────────────────────────────────────────────────
#[derive(Clone, Copy)]
struct Bullet { x: i32, y: i32, alive: bool, damage: u8, color: Rgb565 }
impl Bullet {
    const DEAD: Self = Self { x: 0, y: 0, alive: false, damage: 0, color: Rgb565::BLACK };
}

#[derive(Clone, Copy)]
struct Enemy { x: i32, y: i32, hp: u8, alive: bool }
impl Enemy {
    const DEAD: Self = Self { x: 0, y: 0, hp: 0, alive: false };
    const W: i32 = 12;
    const H: i32 = 10;
}

struct Player { y: i32, weapon_idx: usize, fire_cooldown: u8 }
impl Player {
    const X: i32 = GAME_X + 20;
    const W: i32 = 14;
    const H: i32 = 10;
    fn new() -> Self { Self { y: GAME_H / 2, weapon_idx: 0, fire_cooldown: 0 } }
    fn weapon(&self) -> &'static WeaponConfig { &WEAPONS[self.weapon_idx] }
    fn cycle_weapon(&mut self) { self.weapon_idx = (self.weapon_idx + 1) % WEAPONS.len(); }
}

// ── Game state ──────────────────────────────────────────────────────────────
struct Game {
    player: Player,
    bullets: [Bullet; MAX_BULLETS],
    enemies: [Enemy; MAX_ENEMIES],
    score: u32,
    tick: u32,
    scroll_offset: u16,
    alive: bool,
    rng: Rng,
    enemy_spawn_timer: u8,
}

impl Game {
    fn new() -> Self {
        Self {
            player: Player::new(),
            bullets: [Bullet::DEAD; MAX_BULLETS],
            enemies: [Enemy::DEAD; MAX_ENEMIES],
            score: 0, tick: 0, scroll_offset: 0,
            alive: true, rng: Rng::new(0xDEAD_BEEF), enemy_spawn_timer: 0,
        }
    }

    fn update(&mut self) {
        self.tick += 1;

        if INPUT_UP.load(Ordering::Relaxed) {
            self.player.y = (self.player.y - PLAYER_SPEED).max(Player::H / 2);
        }
        if INPUT_DOWN.load(Ordering::Relaxed) {
            self.player.y = (self.player.y + PLAYER_SPEED).min(GAME_H - Player::H / 2 - 1);
        }

        if self.player.fire_cooldown > 0 { self.player.fire_cooldown -= 1; }
        if INPUT_FIRE.load(Ordering::Relaxed) && self.player.fire_cooldown == 0 {
            let w = self.player.weapon();
            for i in 0..w.count as usize {
                if let Some(slot) = self.bullets.iter_mut().find(|b| !b.alive) {
                    *slot = Bullet {
                        x: Player::X + Player::W, y: self.player.y + w.offsets[i],
                        alive: true, damage: w.damage, color: w.color,
                    };
                }
            }
            self.player.fire_cooldown = FIRE_COOLDOWN;
        }

        for b in &mut self.bullets {
            if b.alive {
                b.x += BULLET_SPEED;
                if b.x > GAME_X + GAME_W { b.alive = false; }
            }
        }

        if self.enemy_spawn_timer == 0 {
            let interval = 60u8.saturating_sub((self.score / 5) as u8).max(20);
            self.enemy_spawn_timer = interval;
            if let Some(slot) = self.enemies.iter_mut().find(|e| !e.alive) {
                let y = (self.rng.range((GAME_H - Enemy::H) as u32) as i32).max(0);
                *slot = Enemy { x: GAME_X + GAME_W, y, hp: ENEMY_HP, alive: true };
            }
        } else {
            self.enemy_spawn_timer -= 1;
        }

        for e in &mut self.enemies {
            if e.alive {
                e.x -= ENEMY_SPEED;
                if e.x + Enemy::W < GAME_X { e.alive = false; }
            }
        }

        for b in &mut self.bullets {
            if !b.alive { continue; }
            for e in &mut self.enemies {
                if !e.alive { continue; }
                if b.x >= e.x && b.x <= e.x + Enemy::W
                    && b.y >= e.y && b.y <= e.y + Enemy::H
                {
                    b.alive = false;
                    if e.hp <= b.damage {
                        e.alive = false;
                        self.score += 1;
                        LED_CHANNEL.try_send(LedEvent::EnemyKill).ok();
                    }
                    else { e.hp -= b.damage; }
                    break;
                }
            }
        }

        let px = Player::X;
        let py = self.player.y - Player::H / 2;
        for e in &self.enemies {
            if !e.alive { continue; }
            if e.x < px + Player::W && e.x + Enemy::W > px
                && e.y < py + Player::H && e.y + Enemy::H > py
            { self.alive = false; break; }
        }

        self.scroll_offset = (self.scroll_offset + SCROLL_SPEED) % SCROLL_AREA;
    }
}

// ── Rendering helpers ───────────────────────────────────────────────────────

/// Per-column background metadata so we can regenerate fire pixels under sprites.
/// Index = framebuffer column offset (0..GAME_W), stores (world_x, bg_frame).
struct BgMap {
    wx: [i32; SCROLL_AREA as usize],
    frame: [i32; SCROLL_AREA as usize],
}

impl BgMap {
    fn new() -> Self {
        Self {
            wx: [0; SCROLL_AREA as usize],
            frame: [0; SCROLL_AREA as usize],
        }
    }

    /// Record that framebuffer column `fb_x` was drawn with these params.
    fn set(&mut self, fb_x: i32, world_x: i32, bg_frame: i32) {
        let idx = (fb_x - GAME_X) as usize;
        if idx < SCROLL_AREA as usize {
            self.wx[idx] = world_x;
            self.frame[idx] = bg_frame;
        }
    }

    /// Get (world_x, bg_frame) for a framebuffer column.
    fn get(&self, fb_x: i32) -> (i32, i32) {
        let idx = (fb_x - GAME_X) as usize;
        (self.wx[idx], self.frame[idx])
    }
}

/// Convert a screen-space X in the game area to the framebuffer X that the
/// hardware scroll will display at that position.
fn screen_to_fb_x(sx: i32, scroll_offset: u16) -> i32 {
    if sx < GAME_X || sx >= GAME_X + GAME_W { return sx; }
    let local = sx - GAME_X;
    GAME_X + (local + scroll_offset as i32) % GAME_W
}

/// Draw a filled rectangle at raw framebuffer coordinates (no scroll compensation).
fn draw_rect_fb(display: &mut Display, x: i32, y: i32, w: i32, h: i32, color: Rgb565) {
    if w <= 0 || h <= 0 { return; }
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + w).min(SCREEN_W);
    let y1 = (y + h).min(SCREEN_H);
    let cw = (x1 - x0) as u32;
    let ch = (y1 - y0) as u32;
    if cw == 0 || ch == 0 { return; }
    Rectangle::new(Point::new(x0, y0), Size::new(cw, ch))
        .into_styled(PrimitiveStyle::with_fill(color))
        .draw(display)
        .unwrap();
}

/// Draw a filled rectangle in screen-space, compensating for hardware scroll.
/// Clamps to the game area — nothing is ever drawn into the HUD regions.
fn draw_rect_scr(display: &mut Display, x: i32, y: i32, w: i32, h: i32, color: Rgb565, so: u16) {
    if w <= 0 || h <= 0 { return; }
    let x0 = x.max(GAME_X);
    let x1 = (x + w).min(GAME_X + GAME_W);
    if x0 >= x1 { return; }
    let fb_start = screen_to_fb_x(x0, so);
    let fb_end = screen_to_fb_x(x1 - 1, so);
    if fb_start <= fb_end {
        draw_rect_fb(display, fb_start, y, fb_end - fb_start + 1, h, color);
    } else {
        draw_rect_fb(display, fb_start, y, GAME_X + GAME_W - fb_start, h, color);
        draw_rect_fb(display, GAME_X, y, fb_end - GAME_X + 1, h, color);
    }
}

fn draw_player(display: &mut Display, py: i32, color: Rgb565, so: u16) {
    let x = Player::X;
    let y = py - Player::H / 2;
    // Fuselage
    draw_rect_scr(display, x + 2, y + 2, 10, 6, color, so);
    // Nose
    draw_rect_scr(display, x + 12, y + 3, 2, 4, color, so);
    // Wings
    draw_rect_scr(display, x, y, 4, 2, color, so);
    draw_rect_scr(display, x, y + Player::H - 2, 4, 2, color, so);
}

fn erase_player(display: &mut Display, py: i32, so: u16, bg: &BgMap) {
    let x = Player::X;
    let y = py - Player::H / 2;
    restore_fire_rect(display, x, y, Player::W, Player::H, so, bg);
}

fn draw_enemy(display: &mut Display, e: &Enemy, color: Rgb565, so: u16) {
    draw_rect_scr(display, e.x + 2, e.y + 1, 8, 8, color, so);
    draw_rect_scr(display, e.x, e.y + 3, 2, 4, color, so);
    draw_rect_scr(display, e.x + 10, e.y + 3, 2, 4, color, so);
    if color != Rgb565::BLACK {
        let eye = if e.hp <= 1 { Rgb565::RED } else { Rgb565::WHITE };
        draw_rect_scr(display, e.x + 4, e.y + 3, 2, 2, eye, so);
        draw_rect_scr(display, e.x + 7, e.y + 3, 2, 2, eye, so);
    }
}

fn erase_enemy(display: &mut Display, e: &Enemy, so: u16, bg: &BgMap) {
    restore_fire_rect(display, e.x, e.y, Enemy::W, Enemy::H, so, bg);
}

fn draw_bullet(display: &mut Display, b: &Bullet, color: Rgb565, so: u16) {
    draw_rect_scr(display, b.x, b.y, 3, 2, color, so);
}

fn erase_bullet(display: &mut Display, b: &Bullet, so: u16, bg: &BgMap) {
    restore_fire_rect(display, b.x, b.y, 3, 2, so, bg);
}

/// Paint a fire-shader column into the framebuffer at raw FB coordinate fb_x.
fn draw_fire_column(display: &mut Display, fb_x: i32, world_x: i32, frame: i32, bg: &mut BgMap) {
    let w = SCROLL_SPEED as u32;
    let area = Rectangle::new(Point::new(fb_x, 0), Size::new(w, GAME_H as u32));
    let pixels = (0..GAME_H as i32).flat_map(|y| {
        let c = fire_bg(world_x, y, frame);
        core::iter::repeat_n(c, w as usize)
    });
    display.fill_contiguous(&area, pixels).unwrap();
    for dx in 0..w as i32 {
        bg.set(fb_x + dx, world_x + dx, frame);
    }
}

/// Fill the entire scrollable background with the fire shader at the given frame.
fn fill_fire_background(display: &mut Display, frame: i32, bg: &mut BgMap) {
    let w = SCROLL_SPEED as usize;
    for col in (GAME_X..(GAME_X + GAME_W)).step_by(w) {
        let wx = col - GAME_X;
        let area = Rectangle::new(Point::new(col, 0), Size::new(w as u32, GAME_H as u32));
        let pixels = (0..GAME_H as i32).flat_map(move |y| {
            let c = fire_bg(wx, y, frame);
            core::iter::repeat_n(c, w)
        });
        display.fill_contiguous(&area, pixels).unwrap();
        for dx in 0..w as i32 {
            bg.set(col + dx, wx + dx, frame);
        }
    }
}

/// Restore fire background for a screen-space rectangle (erase a sprite).
/// Each column is regenerated from the BgMap metadata.
fn restore_fire_rect(display: &mut Display, x: i32, y: i32, w: i32, h: i32, so: u16, bg: &BgMap) {
    if w <= 0 || h <= 0 { return; }
    let x0 = x.max(GAME_X);
    let x1 = (x + w).min(GAME_X + GAME_W);
    let y0 = y.max(0);
    let y1 = (y + h).min(GAME_H);
    if x0 >= x1 || y0 >= y1 { return; }

    // Check if the rect wraps around the scroll boundary
    let fb_start = screen_to_fb_x(x0, so);
    let fb_end = screen_to_fb_x(x1 - 1, so);

    if fb_start <= fb_end {
        // Contiguous in framebuffer — single fill_contiguous call
        let fw = (fb_end - fb_start + 1) as u32;
        let fh = (y1 - y0) as u32;
        let area = Rectangle::new(Point::new(fb_start, y0), Size::new(fw, fh));
        let pixels = (y0..y1).flat_map(|py| {
            (fb_start..=fb_end).map(move |fb_x| {
                let (wx, frame) = bg.get(fb_x);
                fire_bg(wx, py, frame)
            })
        });
        display.fill_contiguous(&area, pixels).unwrap();
    } else {
        // Wraps — two contiguous fills (right part + left part)
        // Right part: fb_start .. GAME_X + GAME_W
        let rw = (GAME_X + GAME_W - fb_start) as u32;
        let fh = (y1 - y0) as u32;
        let area_r = Rectangle::new(Point::new(fb_start, y0), Size::new(rw, fh));
        let pixels_r = (y0..y1).flat_map(|py| {
            (fb_start..GAME_X + GAME_W).map(move |fb_x| {
                let (wx, frame) = bg.get(fb_x);
                fire_bg(wx, py, frame)
            })
        });
        display.fill_contiguous(&area_r, pixels_r).unwrap();

        // Left part: GAME_X .. fb_end + 1
        let lw = (fb_end - GAME_X + 1) as u32;
        let area_l = Rectangle::new(Point::new(GAME_X, y0), Size::new(lw, fh));
        let pixels_l = (y0..y1).flat_map(|py| {
            (GAME_X..=fb_end).map(move |fb_x| {
                let (wx, frame) = bg.get(fb_x);
                fire_bg(wx, py, frame)
            })
        });
        display.fill_contiguous(&area_l, pixels_l).unwrap();
    }
}


fn format_u32(mut n: u32, buf: &mut [u8; 16]) -> &str {
    if n == 0 { buf[0] = b'0'; return unsafe { core::str::from_utf8_unchecked(&buf[..1]) }; }
    let mut i = 0;
    while n > 0 { buf[i] = b'0' + (n % 10) as u8; n /= 10; i += 1; }
    buf[..i].reverse();
    unsafe { core::str::from_utf8_unchecked(&buf[..i]) }
}

/// Right-side HUD (score). Fixed region — no scroll compensation.
fn draw_hud_score(display: &mut Display, score: u32) {
    let hx = SCREEN_W - HUD_RIGHT as i32;
    draw_rect_fb(display, hx, 0, HUD_RIGHT as i32, SCREEN_H, Rgb565::new(0, 0, 4));
    draw_rect_fb(display, hx, 0, 1, SCREEN_H, Rgb565::new(2, 6, 12));
    let lx = hx + 4;
    // "S" label
    draw_rect_fb(display, lx, 4, 5, 1, Rgb565::CSS_LIGHT_GRAY);
    draw_rect_fb(display, lx, 5, 1, 2, Rgb565::CSS_LIGHT_GRAY);
    draw_rect_fb(display, lx, 7, 5, 1, Rgb565::CSS_LIGHT_GRAY);
    draw_rect_fb(display, lx + 4, 8, 1, 2, Rgb565::CSS_LIGHT_GRAY);
    draw_rect_fb(display, lx, 10, 5, 1, Rgb565::CSS_LIGHT_GRAY);
    // Digits
    let mut buf = [0u8; 16];
    let s = format_u32(score, &mut buf);
    for (i, ch) in s.bytes().enumerate() {
        let d = ch - b'0';
        let dy = 18 + i as i32 * 14;
        let b = 8 + d * 2;
        draw_rect_fb(display, lx, dy, 10, 10, Rgb565::new(b, b * 2, b));
        draw_rect_fb(display, lx + 2, dy + 2, 6, 6, Rgb565::new(0, 0, 4));
        draw_rect_fb(display, lx + 3, dy + 3, 4, 4, Rgb565::new(b / 2, b, b));
    }
}

/// Left-side HUD (weapon info). Fixed region — no scroll compensation.
fn draw_hud_weapon(display: &mut Display, weapon: &WeaponConfig) {
    draw_rect_fb(display, 0, 0, HUD_LEFT as i32, SCREEN_H, Rgb565::new(0, 0, 4));
    draw_rect_fb(display, HUD_LEFT as i32 - 1, 0, 1, SCREEN_H, Rgb565::new(2, 6, 12));
    let lx = 4;
    for i in 0..weapon.count as i32 {
        let dy = 8 + i * 14;
        draw_rect_fb(display, lx, dy, 14, 8, weapon.color);
        draw_rect_fb(display, lx + 2, dy + 2, 8, 4, Rgb565::BLACK);
        draw_rect_fb(display, lx + 3, dy + 3, 6, 2, weapon.color);
    }
    let ny = SCREEN_H - 50;
    let style = MonoTextStyle::new(&FONT_6X10, weapon.color);
    for (i, &ch) in weapon.name.iter().enumerate() {
        let dy = ny + i as i32 * 12;
        let hud_bg = Rgb565::new(0, 0, 4);
        draw_rect_fb(display, lx, dy, 14, 12, hud_bg);
        let buf = [ch];
        let s = unsafe { core::str::from_utf8_unchecked(&buf) };
        Text::new(s, Point::new(lx + 4, dy + 9), style)
            .draw(display)
            .unwrap();
    }
}

/// Draw FPS counter in the right HUD (score side), at the bottom.
fn draw_hud_fps(display: &mut Display, fps: u32, delay_ms: u32) {
    let fps = fps.min(99);
    let color = if fps >= 25 { Rgb565::CSS_LIME_GREEN } else { Rgb565::RED };
    let hud_bg = Rgb565::new(0, 0, 4);
    let hx = SCREEN_W - HUD_RIGHT as i32;
    let style = MonoTextStyle::new(&FONT_4X6, color);
    // Frame delay (ms idle at end of frame)
    let dy_delay = SCREEN_H - 16;
    draw_rect_fb(display, hx + 2, dy_delay, 20, 8, hud_bg);
    let mut buf2 = [0u8; 16];
    let ds = format_u32(delay_ms.min(99), &mut buf2);
    Text::new(ds, Point::new(hx + 4, dy_delay + 5), style)
        .draw(display)
        .unwrap();
    // FPS
    let dy = SCREEN_H - 8;
    draw_rect_fb(display, hx + 2, dy, 20, 8, hud_bg);
    let mut buf = [0u8; 16];
    let s = format_u32(fps, &mut buf);
    Text::new(s, Point::new(hx + 4, dy + 5), style)
        .draw(display)
        .unwrap();
}

// ── LED signalling ──────────────────────────────────────────────────────────
#[derive(Clone, Copy)]
enum LedEvent {
    /// Enemy destroyed — white flash
    EnemyKill,
    /// Score changed — update bar (carries score)
    Score(u32),
    /// Game over — red flash then idle
    GameOver,
}

static LED_CHANNEL: Channel<embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex, LedEvent, 4> =
    Channel::new();

// ── Tasks ───────────────────────────────────────────────────────────────────

#[embassy_executor::task]
async fn led_task(leds: &'static mut Leds<'static>) {
    info!("LED task started");
    loop {
        let event = LED_CHANNEL.receive().await;
        match event {
            LedEvent::EnemyKill => {
                // Bright flash fading to black over ~160ms
                for i in (0..=8).rev() {
                    let brightness = i * 5; // MAX flashbang: * 30
                    leds.fill(Srgb::new(brightness, brightness, brightness));
                    leds.update().await;
                    Timer::after(Duration::from_millis(20)).await;
                }
            }
            LedEvent::Score(score) => {
                let lit = ((score as usize).min(BAR_COUNT * 5)) / 5;
                let mut bar = [Srgb::new(0u8, 0, 0); BAR_COUNT];
                for i in 0..lit.min(BAR_COUNT) {
                    bar[i] = Srgb::new(0, 4, 2);
                }
                leds.set_both_bars(&bar);
                leds.update().await;
            }
            LedEvent::GameOver => {
                for _ in 0..3 {
                    leds.fill(Srgb::new(20, 0, 0));
                    leds.update().await;
                    Timer::after(Duration::from_millis(300)).await;
                    leds.clear();
                    leds.update().await;
                    Timer::after(Duration::from_millis(300)).await;
                }
            }
        }
    }
}

#[embassy_executor::task]
async fn input_task(buttons: &'static mut Buttons) {
    info!("Input task started");
    loop {
        INPUT_UP.store(buttons.up.is_low(), Ordering::Relaxed);
        INPUT_DOWN.store(buttons.down.is_low(), Ordering::Relaxed);
        INPUT_FIRE.store(buttons.a.is_low(), Ordering::Relaxed);
        INPUT_START.store(buttons.start.is_low(), Ordering::Relaxed);
        Timer::after(Duration::from_millis(10)).await;
    }
}

#[embassy_executor::task]
async fn game_task(
    display: &'static mut Display<'static>,
    backlight: &'static mut Backlight,
) {
    backlight.on();
    info!("Space shooter started");

    loop {
        display.set_vertical_scroll_region(HUD_RIGHT, HUD_LEFT).unwrap();

        let mut game = Game::new();
        let mut bg_frame: i32 = 0;
        let mut bg = BgMap::new();
        let mut world_x: i32 = GAME_W as i32;

        fill_fire_background(display, bg_frame, &mut bg);

        draw_hud_score(display, 0);
        draw_hud_weapon(display, game.player.weapon());

        let mut prev_player_y = game.player.y;
        let mut prev_weapon_idx = game.player.weapon_idx;
        let mut prev_score = game.score;
        let mut prev_scroll = game.scroll_offset;
        let mut prev_bullets = [Bullet::DEAD; MAX_BULLETS];
        let mut prev_enemies = [Enemy::DEAD; MAX_ENEMIES];

        let tick = Duration::from_millis(TICK_MS);
        let mut next_frame = Instant::now() + tick;
        let mut fps_accum: u32 = 0;
        let mut fps_timer = Instant::now();

        while game.alive {
            let so_old = prev_scroll;

            // Advance game state
            if game.tick % 200 == 0 { game.player.cycle_weapon(); }
            game.update();

            // Update scroll and paint new background column first
            display.set_vertical_scroll_offset(HUD_RIGHT + game.scroll_offset).unwrap();
            let so = game.scroll_offset;
            bg_frame += 1;

            let fb_col = GAME_X + ((so as i32 + GAME_W - SCROLL_SPEED as i32) % GAME_W);
            draw_fire_column(display, fb_col, world_x, bg_frame, &mut bg);
            world_x += SCROLL_SPEED as i32;

            // Erase old bullets (they move in FB space)
            for b in &prev_bullets {
                if b.alive { erase_bullet(display, b, so_old, &bg); }
            }
            // Erase enemies that just died
            for (pe, ne) in prev_enemies.iter().zip(game.enemies.iter()) {
                if pe.alive && !ne.alive {
                    erase_enemy(display, pe, so_old, &bg);
                }
            }
            // Player always needs erase+redraw (fixed screen X, moves in FB space with scroll)
            erase_player(display, prev_player_y, so_old, &bg);
            draw_player(display, game.player.y, Rgb565::CSS_LIME_GREEN, so);

            for b in &game.bullets {
                if b.alive { draw_bullet(display, b, b.color, so); }
            }
            // Enemies are stationary in FB space (ENEMY_SPEED == SCROLL_SPEED),
            // so just overdraw them — no erase needed, no blink.
            for e in &game.enemies {
                if e.alive { draw_enemy(display, e, Rgb565::CSS_TOMATO, so); }
            }

            if game.score != prev_score {
                draw_hud_score(display, game.score);
                LED_CHANNEL.try_send(LedEvent::Score(game.score)).ok();
                prev_score = game.score;
            }
            if game.player.weapon_idx != prev_weapon_idx {
                draw_hud_weapon(display, game.player.weapon());
                prev_weapon_idx = game.player.weapon_idx;
            }

            prev_player_y = game.player.y;
            prev_bullets = game.bullets;
            prev_enemies = game.enemies;
            prev_scroll = so;

            // FPS counter
            fps_accum += 1;
            let now = Instant::now();
            let delay_ms = if next_frame > now {
                (next_frame - now).as_millis() as u32
            } else {
                0
            };
            if now.duration_since(fps_timer).as_millis() >= 1000 {
                let fps = fps_accum;
                fps_accum = 0;
                fps_timer = now;
                draw_hud_fps(display, fps, delay_ms);
            }

            Timer::at(next_frame).await;
            next_frame += tick;
        }

        // Game over — reset scroll so text renders at correct screen positions
        info!("Game over! Score: {}", game.score);
        display.set_vertical_scroll_offset(HUD_RIGHT).unwrap();
        draw_rect_fb(display, GAME_X, 0, GAME_W, GAME_H, Rgb565::BLACK);

        // Box
        draw_rect_fb(display, GAME_X + 50, 40, 172, 90, Rgb565::new(12, 0, 0));
        draw_rect_fb(display, GAME_X + 52, 42, 168, 86, Rgb565::new(4, 0, 0));

        let style = MonoTextStyle::new(&FONT_10X20, Rgb565::RED);
        Text::new("GAME OVER", Point::new(GAME_X + 86, 75), style)
            .draw(display)
            .unwrap();

        let score_style = MonoTextStyle::new(&FONT_10X20, Rgb565::CSS_ORANGE);
        let mut buf = [0u8; 16];
        let s = format_u32(game.score, &mut buf);
        // Center the score: each char is 10px wide
        let sx = GAME_X + 136 - (s.len() as i32 * 10) / 2;
        Text::new("Score:", Point::new(GAME_X + 76, 105), score_style)
            .draw(display)
            .unwrap();
        Text::new(s, Point::new(sx + 70, 105), score_style)
            .draw(display)
            .unwrap();

        LED_CHANNEL.try_send(LedEvent::GameOver).ok();

        loop {
            if INPUT_START.load(Ordering::Relaxed) { break; }
            Timer::after(Duration::from_millis(50)).await;
        }
        Timer::after(Duration::from_millis(200)).await;
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let peripherals = disobey2026badge::init();
    let resources = split_resources!(peripherals);

    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let buttons = mk_static!(Buttons, resources.buttons.into());
    let display = mk_static!(Display<'static>, resources.display.into());
    let backlight = mk_static!(Backlight, resources.backlight.into());
    let leds = mk_static!(Leds<'static>, resources.leds.into());

    spawner.must_spawn(input_task(buttons));
    spawner.must_spawn(led_task(leds));
    spawner.must_spawn(game_task(display, backlight));

    loop {
        Timer::after(Duration::from_secs(600)).await;
    }
}
