//! Demoscene demo — double-buffered, dual-core.
//!
//! Core 0: renders effects into an off-screen framebuffer.
//! Core 1: blits the finished framebuffer to the ST7789 display via SPI/DMA.
//! Two framebuffers swap roles each frame for tear-free output.

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicU8, Ordering};

use defmt::info;
#[allow(clippy::wildcard_imports)]
use disobey2026badge::*;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use embedded_graphics::{
    mono_font::{MonoTextStyle, iso_8859_1::FONT_10X20},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{Line, PrimitiveStyle, Rectangle},
    text::Text,
};
use esp_backtrace as _;
use esp_hal::timer::timg::TimerGroup;
use esp_println as _;

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

const W: i32 = 320;
const H: i32 = 170;
const PIXELS: usize = (W * H) as usize;

// ── Framebuffer ─────────────────────────────────────────────────────────────

/// Minimal DrawTarget backed by a flat pixel array.
struct Fb {
    buf: &'static mut [Rgb565; PIXELS],
}

impl Fb {
    fn clear_black(&mut self) {
        self.buf.fill(Rgb565::BLACK);
    }
}

impl DrawTarget for Fb {
    type Color = Rgb565;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(Point { x, y }, color) in pixels {
            if x >= 0 && x < W && y >= 0 && y < H {
                self.buf[(y * W + x) as usize] = color;
            }
        }
        Ok(())
    }
}

impl OriginDimensions for Fb {
    fn size(&self) -> Size {
        Size::new(W as u32, H as u32)
    }
}

// ── Double-buffer swap protocol ─────────────────────────────────────────────
// 0 = render is working
// 1 = frame is ready for display
// 2 = display is blitting (render waits)

static FRAME_STATE: AtomicU8 = AtomicU8::new(0);

// Single framebuffer — render writes, then display reads. Synchronized
// via FRAME_STATE so they never overlap.
use core::cell::UnsafeCell;

struct SyncBuf(UnsafeCell<[Rgb565; PIXELS]>);
unsafe impl Sync for SyncBuf {}

static FRAMEBUF: SyncBuf = SyncBuf(UnsafeCell::new([Rgb565::BLACK; PIXELS]));

// ── Sine table ──────────────────────────────────────────────────────────────

const SIN_Q: [i16; 65] = [
    0, 3, 6, 9, 12, 16, 19, 22, 25, 28, 31, 34, 37, 40, 43, 46,
    49, 51, 54, 57, 60, 62, 65, 67, 70, 72, 75, 77, 79, 81, 84, 86,
    88, 90, 92, 93, 95, 97, 99, 100, 102, 103, 105, 106, 107, 108, 110, 111,
    112, 113, 114, 114, 115, 116, 117, 117, 118, 118, 119, 119, 119, 120, 120, 120,
    120,
];

fn isin(angle: i32) -> i32 {
    let a = ((angle % 1024) + 1024) as u32 % 1024;
    let quadrant = a / 256;
    let idx = (a % 256) as usize;
    let i = idx * 64 / 256;
    let val = match quadrant {
        0 => SIN_Q[i],
        1 => SIN_Q[64 - i],
        2 => -SIN_Q[i],
        _ => -SIN_Q[64 - i],
    };
    val as i32
}

fn icos(angle: i32) -> i32 {
    isin(angle + 256)
}

fn hash_u32(mut x: u32) -> u32 {
    x = x.wrapping_mul(2654435761);
    x ^= x >> 16;
    x = x.wrapping_mul(0x45d9f3b);
    x ^= x >> 16;
    x
}

// ── Effect 1: Plasma ────────────────────────────────────────────────────────

fn plasma(fb: &mut Fb, frame: u32) {
    let f = frame as i32;
    for y in 0..H {
        let off = (y * W) as usize;
        for x in 0..W {
            let v1 = isin(x * 8 + f * 3);
            let v2 = icos(y * 12 + f * 5);
            let v3 = isin((x + y) * 6 + f * 2);
            let r = ((v1 + v2 + 240) * 31 / 480).clamp(0, 31) as u8;
            let g = ((v2 + v3 + 240) * 63 / 480).clamp(0, 63) as u8;
            let b = ((v1 + v3 + 240) * 31 / 480).clamp(0, 31) as u8;
            fb.buf[off + x as usize] = Rgb565::new(r, g, b);
        }
    }
}

// ── Effect 2: Starfield ─────────────────────────────────────────────────────

struct Star {
    x: i32,
    y: i32,
    speed: i32,
    layer: u8,
}

const NUM_STARS: usize = 40;

fn init_stars(stars: &mut [Star; NUM_STARS]) {
    for i in 0..NUM_STARS {
        let h = hash_u32(i as u32 + 777);
        let layer = (i % 3) as u8;
        stars[i] = Star {
            x: (h % W as u32) as i32,
            y: ((h >> 10) % H as u32) as i32,
            speed: (layer as i32 + 1) * 2,
            layer,
        };
    }
}

fn starfield_frame(fb: &mut Fb, stars: &mut [Star; NUM_STARS], frame: u32) {
    fb.clear_black();
    for star in stars.iter_mut() {
        // Draw trail behind the star
        let dim = match star.layer {
            0 => Rgb565::new(2, 4, 6),
            1 => Rgb565::new(4, 10, 14),
            _ => Rgb565::new(8, 18, 24),
        };
        let trail = star.speed * 3;
        for t in 1..=trail {
            let tx = star.x + t;
            if tx >= 0 && tx < W && star.y >= 0 && star.y < H {
                fb.buf[(star.y * W + tx) as usize] = dim;
            }
        }
        // Bright star head
        let bright = match star.layer {
            0 => Rgb565::new(12, 24, 28),
            1 => Rgb565::new(20, 44, 28),
            _ => Rgb565::WHITE,
        };
        if star.x >= 0 && star.x < W && star.y >= 0 && star.y < H {
            fb.buf[(star.y * W + star.x) as usize] = bright;
        }
        star.x -= star.speed;
        if star.x < -10 {
            let h = hash_u32(frame.wrapping_add(star.y as u32).wrapping_mul(31));
            star.x = W - 1;
            star.y = (h % H as u32) as i32;
        }
    }
}

// ── Effect 3: Copper bars ───────────────────────────────────────────────────

fn copper_bars(fb: &mut Fb, frame: u32) {
    fb.clear_black();
    let bar_h = 12i32;
    for bar in 0..5u32 {
        let phase = frame as i32 * 3 + bar as i32 * 180;
        let y_center = H / 2 + isin(phase) * (H / 2 - bar_h) / 120;
        for row in 0..bar_h {
            let y = y_center + row - bar_h / 2;
            if y < 0 || y >= H { continue; }
            let dist = (row - bar_h / 2).abs();
            let intensity = (31 - dist * 5).max(0);
            let off = (y * W) as usize;
            for x in 0..W {
                let shimmer = isin(x * 20 + phase) * 4 / 120;
                let i = (intensity + shimmer).clamp(0, 31) as u8;
                fb.buf[off + x as usize] = match bar % 5 {
                    0 => Rgb565::new(i, i / 2, 0),
                    1 => Rgb565::new(0, i * 2, i),
                    2 => Rgb565::new(i, 0, i),
                    3 => Rgb565::new(i / 2, i * 2, i / 2),
                    _ => Rgb565::new(i, i * 2, i),
                };
            }
        }
    }
}

// ── Effect 4: Sine scroller ─────────────────────────────────────────────────

const SCROLL_MSG: &[u8] = b"DISOBEY 2026 ** GREETINGS TO ALL HACKERS AND MAKERS ** LOVE YOU ALL <3";

fn sine_scroller(fb: &mut Fb, frame: u32, scroll_x: &mut i32) {
    let style = MonoTextStyle::new(&FONT_10X20, Rgb565::CSS_YELLOW);
    let char_w = 10i32;
    let char_h = 20i32;
    let f = frame as i32;

    for (i, &ch) in SCROLL_MSG.iter().enumerate() {
        let x = i as i32 * char_w + *scroll_x;
        if x < -char_w || x >= W { continue; }
        let wave = isin(x * 3 + f * 6) * 30 / 120;
        let y = H / 2 + wave;
/*        Rectangle::new(
            Point::new(x, y - char_h + 4),
            Size::new(char_w as u32, char_h as u32 + 1),
        )
        .into_styled(PrimitiveStyle::with_fill(Rgb565::new(1, 2, 2)))
        .draw(fb)
        .unwrap(); */
        let buf = [ch];
        if let Ok(s) = core::str::from_utf8(&buf) {
            Text::new(s, Point::new(x, y), style).draw(fb).unwrap();
        }
    }
    *scroll_x -= 3;
    let total_w = SCROLL_MSG.len() as i32 * char_w;
    if *scroll_x < -total_w { *scroll_x = W; }
}

// ── Effect 5: Rotozoom ──────────────────────────────────────────────────────

fn rotozoom(fb: &mut Fb, frame: u32) {
    let f = frame as i32;
    let sa = isin(f * 2);
    let ca = icos(f * 2);
    let zoom = (80 + isin(f * 3) * 60 / 120).max(20);
    for y in 0..H {
        let off = (y * W) as usize;
        let yc = y - H / 2;
        for x in 0..W {
            let xc = x - W / 2;
            let u = (xc * ca - yc * sa) / zoom;
            let v = (xc * sa + yc * ca) / zoom;
            let tex = (u ^ v).unsigned_abs() & 0xF;
            let c = (tex * 2) as u8;
            fb.buf[off + x as usize] = Rgb565::new(c.min(31), (c / 2).min(31), c.min(31));
        }
    }
}

// ── Effect 6: Wireframe cube ────────────────────────────────────────────────

const CUBE_VERTS: [[i32; 3]; 8] = [
    [-1, -1, -1], [ 1, -1, -1], [ 1,  1, -1], [-1,  1, -1],
    [-1, -1,  1], [ 1, -1,  1], [ 1,  1,  1], [-1,  1,  1],
];

const CUBE_EDGES: [[usize; 2]; 12] = [
    [0,1],[1,2],[2,3],[3,0],
    [4,5],[5,6],[6,7],[7,4],
    [0,4],[1,5],[2,6],[3,7],
];

fn project(v: [i32; 3], ax: i32, ay: i32, az: i32, scale: i32) -> (i32, i32) {
    let (mut x, mut y, mut z) = (v[0] * scale, v[1] * scale, v[2] * scale);
    let (ny, nz) = ((y * icos(ax) - z * isin(ax)) / 120, (y * isin(ax) + z * icos(ax)) / 120);
    y = ny; z = nz;
    let (nx, nz2) = ((x * icos(ay) + z * isin(ay)) / 120, (-x * isin(ay) + z * icos(ay)) / 120);
    x = nx; z = nz2;
    let (nx2, ny2) = ((x * icos(az) - y * isin(az)) / 120, (x * isin(az) + y * icos(az)) / 120);
    x = nx2; y = ny2;
    let d = (z + 500).max(50);
    (W / 2 + x * 200 / d, H / 2 + y * 200 / d)
}

fn wireframe_cube(fb: &mut Fb, frame: u32) {
    fb.clear_black();
    let f = frame as i32;
    let (ax, ay, az, sz) = (f * 3, f * 5, f * 2, 60);
    for &[a, b] in &CUBE_EDGES {
        let (x1, y1) = project(CUBE_VERTS[a], ax, ay, az, sz);
        let (x2, y2) = project(CUBE_VERTS[b], ax, ay, az, sz);
        Line::new(Point::new(x1, y1), Point::new(x2, y2))
            .into_styled(PrimitiveStyle::with_stroke(Rgb565::new(0, 63, 8), 1))
            .draw(fb).unwrap();
    }
}

// ── Effect 7: Tunnel ────────────────────────────────────────────────────────

fn tunnel(fb: &mut Fb, frame: u32) {
    let f = frame as i32;
    for y in 0..H {
        let off = (y * W) as usize;
        let dy = y - H / 2;
        for x in 0..W {
            let dx = x - W / 2;
            let dist = {
                let (ax, ay) = (dx.abs(), dy.abs());
                let (mx, mn) = if ax > ay { (ax, ay) } else { (ay, ax) };
                mx + mn / 2
            };
            if dist < 2 {
                fb.buf[off + x as usize] = Rgb565::BLACK;
                continue;
            }
            let angle = {
                let a = if dx.abs() > dy.abs() {
                    256 * dy / dx.abs()
                } else if dy != 0 {
                    512 - 256 * dx / dy.abs()
                } else { 0 };
                if dx < 0 { 512 - a } else { a }
            };
            let u = (1200 / dist + f * 3) & 0x1F;
            let v = (angle / 8 + f) & 0x1F;
            let tex = (u ^ v) as u8;
            fb.buf[off + x as usize] = Rgb565::new(tex.min(31), (tex / 2).min(31), (tex * 2).min(31));
        }
    }
}

// ── Effect 8: Warp checkerboard ─────────────────────────────────────────────

fn warp_checker(fb: &mut Fb, frame: u32) {
    let f = frame as i32;
    for y in 0..H {
        let off = (y * W) as usize;
        let dy = y - H / 2;
        for x in 0..W {
            let dx = x - W / 2;
            let dist = {
                let (ax, ay) = (dx.abs(), dy.abs());
                let (mx, mn) = if ax > ay { (ax, ay) } else { (ay, ax) };
                mx + mn / 2
            };
            let warp = isin(dist * 6 + f * 8) * 15 / 120;
            let wx = (x + warp) / 20;
            let wy = (y + warp) / 20;
            fb.buf[off + x as usize] = if (wx + wy) & 1 == 0 {
                Rgb565::new(28, 10, 0)
            } else {
                Rgb565::new(0, 8, 28)
            };
        }
    }
}

// ── Display task (runs on core 1) ───────────────────────────────────────────
// Waits for render to signal a frame is ready, then blits it to the display.

#[embassy_executor::task]
async fn display_blit_task(display: &'static mut Display<'static>) {
    info!("Display blit task running on core 1");
    loop {
        if FRAME_STATE.load(Ordering::Acquire) == 1 {
            // Mark as blitting
            FRAME_STATE.store(2, Ordering::Release);
            // Safety: render is waiting, so we have exclusive read access
            let src: &[Rgb565; PIXELS] = unsafe { &*FRAMEBUF.0.get() };
            let area = Rectangle::new(Point::zero(), Size::new(W as u32, H as u32));
            display.fill_contiguous(&area, src.iter().copied()).unwrap();
            // Done — render can proceed
            FRAME_STATE.store(0, Ordering::Release);
        } else {
            Timer::after(Duration::from_millis(1)).await;
        }
    }
}

// ── Render task (runs on core 0) ────────────────────────────────────────────

const EFFECT_FRAMES: u32 = 200;
const NUM_EFFECTS: u32 = 7;

#[embassy_executor::task]
async fn render_task() {
    info!("Render task running on core 0");

    let mut frame: u32 = 0;
    let mut scroll_x: i32 = W;
    let mut stars = [const { Star { x: 0, y: 0, speed: 1, layer: 0 } }; NUM_STARS];
    init_stars(&mut stars);
    let mut prev_effect: u32 = u32::MAX;

    loop {
        // Wait until display has finished blitting the previous frame
        while FRAME_STATE.load(Ordering::Acquire) != 0 {
            Timer::after(Duration::from_millis(1)).await;
        }

        // Safety: display is idle, we have exclusive write access
        let fb_buf: &'static mut [Rgb565; PIXELS] = unsafe { &mut *FRAMEBUF.0.get() };
        let fb = &mut Fb { buf: fb_buf };

        let effect = (frame / EFFECT_FRAMES) % NUM_EFFECTS;

        if effect != prev_effect {
            if effect == 1 { init_stars(&mut stars); }
            let name = match effect {
                0 => "PLASMA", 1 => "STARFIELD", 2 => "COPPER",
                3 => "ROTOZOOM", 4 => "CUBE", 5 => "TUNNEL", _ => "WARP",
            };
            info!("Effect: {}", name);
            prev_effect = effect;
        }

        // Render current effect
        match effect {
            0 => plasma(fb, frame),
            1 => starfield_frame(fb, &mut stars, frame),
            2 => copper_bars(fb, frame),
            3 => rotozoom(fb, frame),
            4 => wireframe_cube(fb, frame),
            5 => tunnel(fb, frame),
            _ => warp_checker(fb, frame),
        }

        // Sine scroller always on top
        sine_scroller(fb, frame, &mut scroll_x);

        // Signal display task: frame is ready
        FRAME_STATE.store(1, Ordering::Release);
        frame = frame.wrapping_add(1);
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

    // Start second core for the display blit task
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
            // Core 1's main thread — start an executor and run the display task
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

    // Core 0: render task
    spawner.must_spawn(render_task());

    loop {
        Timer::after(Duration::from_secs(600)).await;
    }
}
