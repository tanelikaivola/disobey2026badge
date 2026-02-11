//! Shader-style display: streams pixels directly to the display from a
//! function `(x, y, frame) -> Rgb565`, no framebuffer needed.

#![no_std]
#![no_main]

use defmt::info;
#[allow(clippy::wildcard_imports)]
use disobey2026badge::*;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use embedded_graphics::{pixelcolor::Rgb565, prelude::*, primitives::Rectangle};
use esp_backtrace as _;
use esp_hal::timer::timg::TimerGroup;
use esp_println as _;

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

const W: u32 = 320;
const H: u32 = 170;

// ── Sine table (fixed-point, 0..1023 → -120..120) ──────────────────────────

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

fn icos(angle: i32) -> i32 {
    isin(angle + 256)
}

// ── Shader functions ────────────────────────────────────────────────────────

/// Plasma — each color channel scrolls in a different direction, high contrast.
fn plasma(x: u32, y: u32, frame: u32) -> Rgb565 {
    let (x, y, f) = (x as i32, y as i32, frame as i32);

    // Three waves per channel but reuse some across channels for speed.
    // Total: 4 isin/icos calls instead of 6.
    let a = isin(x * 10 + f * 7);
    let b = icos(y * 14 - f * 9);
    let c = isin((x - y * 2) * 6 - f * 11);
    let d = icos((x * 3 + y) * 4 + f * 5);

    // Mix differently per channel so they drift apart
    let r = ((a + c) * 31 / 240 + 16).clamp(0, 31) as u8;
    let g = ((b + d) * 63 / 240 + 32).clamp(0, 63) as u8;
    let b = ((c + b) * 31 / 240 + 16).clamp(0, 31) as u8;

    Rgb565::new(r, g, b)
}

/// Tunnel / wormhole.
fn tunnel(x: u32, y: u32, frame: u32) -> Rgb565 {
    let (dx, dy, f) = (x as i32 - W as i32 / 2, y as i32 - H as i32 / 2, frame as i32);
    let (ax, ay) = (dx.abs(), dy.abs());
    let dist = if ax > ay { ax + ay / 2 } else { ay + ax / 2 };
    if dist < 2 {
        return Rgb565::BLACK;
    }
    let angle = if dx.abs() > dy.abs() {
        let a = 256 * dy / dx.abs();
        if dx < 0 { 512 - a } else { a }
    } else if dy != 0 {
        let a = 512 - 256 * dx / dy.abs();
        if dx < 0 { 512 - a } else { a }
    } else {
        0
    };
    let u = (1200 / dist + f * 3) & 0x1F;
    let v = (angle / 8 + f) & 0x1F;
    let tex = (u ^ v) as u8;
    Rgb565::new(tex.min(31), (tex / 2).min(31), (tex * 2).min(31))
}

/// Rotozoom checkerboard.
fn rotozoom(x: u32, y: u32, frame: u32) -> Rgb565 {
    let f = frame as i32;
    let (xc, yc) = (x as i32 - W as i32 / 2, y as i32 - H as i32 / 2);
    let sa = isin(f * 2);
    let ca = icos(f * 2);
    let zoom = (80 + isin(f * 3) * 60 / 120).max(20);
    let u = (xc * ca - yc * sa) / zoom;
    let v = (xc * sa + yc * ca) / zoom;
    let tex = (u ^ v).unsigned_abs() & 0xF;
    let c = (tex * 2) as u8;
    Rgb565::new(c.min(31), (c / 2).min(31), c.min(31))
}

/// Twisting tower — a vertical column that twists and rotates over time.
/// Each row is a horizontal slice through a rotating square cross-section.
fn tower(x: u32, y: u32, frame: u32) -> Rgb565 {
    let f = frame as i32;
    let (xc, yc) = (x as i32 - W as i32 / 2, y as i32 - H as i32 / 2);

    // Twist angle increases with y (vertical twist) and animates with frame
    let twist = yc * 3 + f * 4;
    let sa = isin(twist);
    let ca = icos(twist);

    // Rotate the x coordinate around the tower axis
    let rx = (xc * ca - yc * sa / 4) / 120;
    let ry = (xc * sa + yc * ca / 4) / 120;

    // Tower cross-section: square with side length that pulses
    let size = 40 + isin(f * 2) * 10 / 120;
    let on_tower = rx.abs() < size && ry.abs() < size;

    if !on_tower {
        // Background: dark gradient
        let bg = ((yc + H as i32 / 2) * 4 / H as i32).clamp(0, 4) as u8;
        return Rgb565::new(bg, bg, bg + 2);
    }

    // Face shading based on which side of the square we're on
    let face = if rx.abs() > ry.abs() {
        if rx > 0 { 0 } else { 1 }
    } else {
        if ry > 0 { 2 } else { 3 }
    };

    // Stripe pattern along the tower height
    let stripe = ((yc + f / 2) / 8) & 1;

    match face {
        0 => {
            if stripe == 0 {
                Rgb565::new(24, 12, 4)
            } else {
                Rgb565::new(18, 8, 2)
            }
        }
        1 => {
            if stripe == 0 {
                Rgb565::new(8, 4, 16)
            } else {
                Rgb565::new(5, 2, 12)
            }
        }
        2 => {
            if stripe == 0 {
                Rgb565::new(4, 20, 10)
            } else {
                Rgb565::new(2, 14, 6)
            }
        }
        _ => {
            if stripe == 0 {
                Rgb565::new(6, 10, 24)
            } else {
                Rgb565::new(3, 6, 18)
            }
        }
    }
}

/// Copper bars — horizontal metallic bands that bounce vertically.
fn copper(x: u32, y: u32, frame: u32) -> Rgb565 {
    let (x, y, f) = (x as i32, y as i32, frame as i32);
    let mut r = 0i32;
    let mut g = 0i32;
    let mut b = 0i32;
    // 5 bars, each a different color, bouncing at different speeds
    for bar in 0..5i32 {
        let phase = f * (3 + bar) + bar * 200;
        let center = H as i32 / 2 + isin(phase) * (H as i32 / 2 - 10) / 120;
        let dist = (y - center).abs();
        if dist < 12 {
            let intensity = (12 - dist) * 3;
            let shimmer = isin(x * 20 + phase) * intensity / 480;
            let i = (intensity + shimmer).max(0);
            match bar % 5 {
                0 => { r += i; }
                1 => { g += i; b += i / 2; }
                2 => { r += i; b += i; }
                3 => { g += i; }
                _ => { r += i / 2; g += i; b += i; }
            }
        }
    }
    Rgb565::new(r.clamp(0, 31) as u8, g.clamp(0, 63) as u8, b.clamp(0, 31) as u8)
}

/// Fire — rising flame effect using pseudo-random hash.
fn fire(x: u32, y: u32, frame: u32) -> Rgb565 {
    let (x, y, f) = (x as i32, y as i32, frame as i32);
    // Invert y so flames rise from the bottom
    let fy = H as i32 - 1 - y;
    // Sample noise at multiple scales for turbulence
    let n1 = isin(x * 7 + fy * 3 - f * 8);
    let n2 = icos(x * 3 + fy * 9 - f * 12);
    let n3 = isin((x + fy) * 5 - f * 6);
    let heat = (n1 + n2 + n3 + 360) * fy / (H as i32 * 3);
    let heat = heat.clamp(0, 120);
    // Map heat to fire palette: black → red → orange → yellow → white
    let r = (heat * 31 / 40).clamp(0, 31) as u8;
    let g = ((heat - 30).max(0) * 63 / 60).clamp(0, 63) as u8;
    let b = ((heat - 80).max(0) * 31 / 40).clamp(0, 31) as u8;
    Rgb565::new(r, g, b)
}

/// Matrix rain — falling green columns.
fn matrix(x: u32, y: u32, frame: u32) -> Rgb565 {
    let (x, y, f) = (x as i32, y as i32, frame as i32);
    // Each column has its own speed and phase derived from a hash
    let col_hash = ((x.wrapping_mul(2654435761u32 as i32)) ^ (x * 31337)) as u32;
    let speed = 3 + (col_hash % 5) as i32;
    let phase = (col_hash / 5 % 170) as i32;
    let head = (f * speed / 2 + phase) % (H as i32 + 40);
    let dist = head - y;
    if dist < 0 || dist > 30 {
        // Background: very faint green noise
        let noise = ((col_hash.wrapping_add(y as u32 * 7919)) % 8) as u8;
        return Rgb565::new(0, noise / 2, 0);
    }
    if dist == 0 {
        // Bright white head
        Rgb565::new(20, 63, 20)
    } else {
        // Fading green tail
        let g = (63 - dist * 2).clamp(4, 63) as u8;
        Rgb565::new(0, g, 0)
    }
}

/// Ripple — concentric rings expanding from center with interference.
fn ripple(x: u32, y: u32, frame: u32) -> Rgb565 {
    let f = frame as i32;
    let (dx, dy) = (x as i32 - W as i32 / 2, y as i32 - H as i32 / 2);
    let dist = {
        // Use proper-ish distance (avoid sqrt with the Chebyshev trick)
        let (ax, ay) = (dx.abs(), dy.abs());
        let (mx, mn) = if ax > ay { (ax, ay) } else { (ay, ax) };
        mx + mn * 3 / 8
    };
    // Two ring sources at different speeds
    let w1 = isin(dist * 8 - f * 10);
    let w2 = isin(dist * 6 + f * 7);
    let v = (w1 + w2 + 240) / 2;
    let r = (v * 20 / 240).clamp(0, 31) as u8;
    let g = (v * 40 / 240).clamp(0, 63) as u8;
    let b = (v * 31 / 240).clamp(0, 31) as u8;
    Rgb565::new(r, g, b)
}

/// Ray marching — sphere hovering over a checkered ground plane.
/// All positions in plain integer world units (1 unit ≈ 1 pixel at mid-depth).
fn raymarch(x: u32, y: u32, frame: u32) -> Rgb565 {
    let f = frame as i32;

    // Ray direction: screen-centered, scaled ×64 for precision
    let rdx = x as i32 - W as i32 / 2;
    let rdy = H as i32 / 2 - y as i32;
    let rdz: i32 = 160; // focal length

    // Sphere: orbits, bobs
    let sx = isin(f * 3) * 80 / 120;
    let sy = 60 + isin(f * 5) * 20 / 120;
    let sz: i32 = 300 + icos(f * 3) * 80 / 120;
    let sr: i32 = 40;

    // March along ray from origin (0, 50, 0) — camera slightly above ground
    let mut px: i32 = isin(f * 2) / 60; // slight sway
    let mut py: i32 = 50;
    let mut pz: i32 = 0;

    // We step in direction (rdx, rdy, rdz) but need to normalize step size.
    // Approximate ray length for step scaling
    let rlen = isqrt_i(rdx * rdx + rdy * rdy + rdz * rdz).max(1);

    let mut hit = 0u8;
    for _ in 0..40 {
        // SDF: sphere
        let spx = px - sx;
        let spy = py - sy;
        let spz = pz - sz;
        let sd = isqrt_i(spx * spx + spy * spy + spz * spz) - sr;

        // SDF: ground plane at y=0
        let gd = py;

        let d = sd.min(gd);

        if d < 2 {
            hit = if sd < gd { 1 } else { 2 };
            break;
        }

        // Step: move d units along the ray
        px += rdx * d / rlen;
        py += rdy * d / rlen;
        pz += rdz * d / rlen;

        if pz > 600 {
            break;
        }
    }

    match hit {
        1 => {
            // Sphere normal
            let nx = px - sx;
            let ny = py - sy;
            let nz = pz - sz;
            let nl = isqrt_i(nx * nx + ny * ny + nz * nz).max(1);
            // Light from upper-right-front: (1, 2, 1) / ~2.4
            let dot = (nx + ny * 2 + nz) * 120 / (nl * 3);
            let i = dot.clamp(8, 120);
            Rgb565::new(
                (i * 22 / 120).clamp(0, 31) as u8,
                (i * 55 / 120).clamp(0, 63) as u8,
                (i * 30 / 120).clamp(0, 31) as u8,
            )
        }
        2 => {
            // Ground checkerboard
            let cx = ((px + 1000) / 40) & 1;
            let cz = ((pz + 1000) / 40) & 1;
            let fog = (pz * 20 / 600).clamp(0, 16) as u8;
            if (cx ^ cz) == 0 {
                Rgb565::new(20 - fog, (44 - fog * 2).max(2) as u8, 10 - fog / 2)
            } else {
                Rgb565::new(8 - fog / 2, (18 - fog).max(2) as u8, 5 - fog / 3)
            }
        }
        _ => {
            let g = (y as i32 * 6 / H as i32).clamp(0, 6) as u8;
            Rgb565::new(g / 2, g, g + 5)
        }
    }
}

/// Integer square root (Babylonian method, 4 iterations).
fn isqrt_i(x: i32) -> i32 {
    if x <= 0 { return 0; }
    let mut g = if x > 10000 { x / 200 + 50 } else { x / 20 + 5 };
    g = (g + x / g) / 2;
    g = (g + x / g) / 2;
    g = (g + x / g) / 2;
    g = (g + x / g) / 2;
    g.max(1)
}

/// Voronoi — animated cells with colored regions.
fn voronoi(x: u32, y: u32, frame: u32) -> Rgb565 {
    const NUM_POINTS: usize = 12;
    let f = frame as i32;
    let (px, py) = (x as i32, y as i32);

    let mut min_d = i32::MAX;
    let mut min2_d = i32::MAX;
    let mut closest = 0u32;

    for i in 0..NUM_POINTS as i32 {
        // Each point orbits on its own path using isin/icos with unique phases
        let phase = i * 83;
        let cx = W as i32 / 2 + isin(f * (2 + i % 3) + phase * 7) * (W as i32 / 3) / 120;
        let cy = H as i32 / 2 + icos(f * (3 + i % 4) + phase * 11) * (H as i32 / 3) / 120;
        let dx = px - cx;
        let dy = py - cy;
        let d = dx * dx + dy * dy;
        if d < min_d {
            min2_d = min_d;
            min_d = d;
            closest = i as u32;
        } else if d < min2_d {
            min2_d = d;
        }
    }

    // Edge detection: difference between closest and second-closest
    let edge = min2_d - min_d;
    if edge < 120 {
        return Rgb565::new(2, 4, 2);
    }

    // Color each cell based on its index
    let hue = (closest * 97 + 30) % 360;
    let (h, s) = (hue as i32, 120i32);
    let c = s;
    let x2 = c * (120 - ((h * 120 / 60) % 240 - 120).abs()) / 120;
    let (r, g, b) = match h / 60 {
        0 => (c, x2, 0),
        1 => (x2, c, 0),
        2 => (0, c, x2),
        3 => (0, x2, c),
        4 => (x2, 0, c),
        _ => (c, 0, x2),
    };

    // Shade by distance from cell center
    let shade = (120 - min_d.min(12000) * 60 / 12000).max(30);
    Rgb565::new(
        (r * shade / 120 * 31 / 120).clamp(2, 31) as u8,
        (g * shade / 120 * 63 / 120).clamp(2, 63) as u8,
        (b * shade / 120 * 31 / 120).clamp(2, 31) as u8,
    )
}

/// Julia set — animated c parameter orbits slowly, colored by escape iteration.
fn julia(x: u32, y: u32, frame: u32) -> Rgb565 {
    let f = frame as i32;
    // Map pixel to complex plane ×1024: x → [-2048, 2048], y aspect-corrected
    let mut zr = (x as i32 - W as i32 / 2) * 4096 / W as i32;
    let mut zi = (y as i32 - H as i32 / 2) * 4096 / W as i32;

    // Animate c on a slow orbit through interesting Julia regions (×1024 scale)
    // c orbits around (-0.5, 0.5) with radius ~0.3
    // isin returns -120..120, so isin(f) * 307 / 120 gives -307..307 (≈ ±0.3 in ×1024)
    let cr = -512 + isin(f) * 307 / 120;       // roughly -0.8 .. -0.2
    let ci = 512 + icos(f * 3 / 4) * 307 / 120; // roughly  0.2 ..  0.8

    let max_iter = 24i32;
    let mut iter = 0i32;
    while iter < max_iter {
        let rr = zr * zr / 1024;
        let ii = zi * zi / 1024;
        if rr + ii > 4 * 1024 {
            break;
        }
        let new_zr = rr - ii + cr;
        zi = 2 * zr * zi / 1024 + ci;
        zr = new_zr;
        iter += 1;
    }

    if iter == max_iter {
        Rgb565::BLACK
    } else {
        let t = iter * 120 / max_iter;
        let r = (isin(t * 8 + 200) + 120) * 31 / 240;
        let g = (isin(t * 6 + 400) + 120) * 63 / 240;
        let b = (icos(t * 10) + 120) * 31 / 240;
        Rgb565::new(r.clamp(0, 31) as u8, g.clamp(0, 63) as u8, b.clamp(0, 31) as u8)
    }
}

/// Warped checkerboard.
fn warp(x: u32, y: u32, frame: u32) -> Rgb565 {
    let (x, y, f) = (x as i32, y as i32, frame as i32);
    let (dx, dy) = (x - W as i32 / 2, y - H as i32 / 2);
    let dist = {
        let (ax, ay) = (dx.abs(), dy.abs());
        if ax > ay { ax + ay / 2 } else { ay + ax / 2 }
    };
    let w = isin(dist * 6 + f * 8) * 15 / 120;
    let wx = (x + w) / 20;
    let wy = (y + w) / 20;
    if (wx + wy) & 1 == 0 {
        Rgb565::new(28, 10, 0)
    } else {
        Rgb565::new(0, 8, 28)
    }
}

// ── Streaming draw ──────────────────────────────────────────────────────────

/// Streams pixels from a shader function directly to the display, no buffer.
fn draw_shader(
    display: &mut Display,
    frame: u32,
    shader: fn(u32, u32, u32) -> Rgb565,
) {
    let area = Rectangle::new(Point::zero(), Size::new(W, H));
    let pixels = (0..(W * H)).map(|i| shader(i % W, i / W, frame));
    display.fill_contiguous(&area, pixels).unwrap();
}

// ── Main ────────────────────────────────────────────────────────────────────

#[embassy_executor::task]
async fn display_task(
    display: &'static mut Display<'static>,
    backlight: &'static mut Backlight,
) {
    info!("Shader demo started");
    backlight.on();

    let shaders: [fn(u32, u32, u32) -> Rgb565; _] = [
        julia, plasma, tunnel, rotozoom, tower, copper, fire, matrix, ripple, raymarch, voronoi, warp,
    ];
    let effect_duration = Duration::from_secs(8);
    let mut frame: u32 = 0;
    let mut idx: usize = 0;
    let mut effect_start = embassy_time::Instant::now();

    loop {
        if embassy_time::Instant::now() - effect_start >= effect_duration {
            idx = (idx + 1) % shaders.len();
            effect_start = embassy_time::Instant::now();
        }
        draw_shader(display, frame, shaders[idx]);
        frame = frame.wrapping_add(1);
        // Yield so the executor can breathe
        Timer::after(Duration::from_millis(1)).await;
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
