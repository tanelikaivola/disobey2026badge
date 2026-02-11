//! Cycles through various test patterns on the ST7789 display forever.

#![no_std]
#![no_main]

use defmt::info;
#[allow(clippy::wildcard_imports)]
use disobey2026badge::*;
use embassy_executor::Spawner;
use embassy_time::{
    Duration,
    Timer,
};
use embedded_graphics::{
    mono_font::{
        MonoTextStyle,
        iso_8859_1::{FONT_10X20, FONT_6X10},
    },
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{
        Circle,
        Line,
        PrimitiveStyle,
        Rectangle,
    },
    text::Text,
};
use esp_backtrace as _;
use esp_hal::timer::timg::TimerGroup;
use esp_println as _;

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

const W: u32 = 320;
const H: u32 = 170;
const PAUSE_MS: u64 = 2000;

fn fill(display: &mut Display, color: Rgb565) {
    let area = Rectangle::new(Point::zero(), Size::new(W, H));
    area.into_styled(PrimitiveStyle::with_fill(color))
        .draw(display)
        .unwrap();
}

/// Solid color fills: red, green, blue, white, black
fn pattern_solid_colors(display: &mut Display) {
    for &(color, name) in &[
        (Rgb565::RED, "Red"),
        (Rgb565::GREEN, "Green"),
        (Rgb565::BLUE, "Blue"),
        (Rgb565::WHITE, "White"),
        (Rgb565::BLACK, "Black"),
    ] {
        fill(display, color);
        let text_color = if color == Rgb565::WHITE {
            Rgb565::BLACK
        } else {
            Rgb565::WHITE
        };
        let style = MonoTextStyle::new(&FONT_10X20, text_color);
        Text::new(name, Point::new(130, 90), style)
            .draw(display)
            .unwrap();
        embassy_time::block_for(Duration::from_millis(PAUSE_MS));
    }
}

/// Vertical color bars (8 bars)
fn pattern_color_bars(display: &mut Display) {
    fill(display, Rgb565::BLACK);
    let colors = [
        Rgb565::WHITE,
        Rgb565::CSS_YELLOW,
        Rgb565::CYAN,
        Rgb565::GREEN,
        Rgb565::CSS_PURPLE,
        Rgb565::RED,
        Rgb565::BLUE,
        Rgb565::BLACK,
    ];
    let bar_w = W / colors.len() as u32;
    for (i, &color) in colors.iter().enumerate() {
        Rectangle::new(
            Point::new((i as u32 * bar_w) as i32, 0),
            Size::new(bar_w, H),
        )
        .into_styled(PrimitiveStyle::with_fill(color))
        .draw(display)
        .unwrap();
    }
}

/// Horizontal gradient from black to white
fn pattern_gradient(display: &mut Display) {
    for x in 0..W {
        let v = ((x as f32 / W as f32) * 31.0) as u8;
        let color = Rgb565::new(v, v * 2, v);
        Rectangle::new(Point::new(x as i32, 0), Size::new(1, H))
            .into_styled(PrimitiveStyle::with_fill(color))
            .draw(display)
            .unwrap();
    }
}

/// RGB gradient: red left-to-right, blue top-to-bottom
fn pattern_rgb_gradient(display: &mut Display) {
    let pixels = (0u32..(W * H)).map(|i| {
        let x = i % W;
        let y = i / W;
        Rgb565::new(
            ((x as f32 / W as f32) * 31.0) as u8,
            0,
            ((y as f32 / H as f32) * 31.0) as u8,
        )
    });
    let area = Rectangle::new(Point::zero(), Size::new(W, H));
    display.fill_contiguous(&area, pixels).unwrap();
}

/// Split screen: color bars on top half, grayscale gradient on bottom half
fn pattern_split_gradient(display: &mut Display) {
    let half = H / 2;
    let bar_colors = [
        Rgb565::WHITE,
        Rgb565::CSS_YELLOW,
        Rgb565::CYAN,
        Rgb565::GREEN,
        Rgb565::CSS_PURPLE,
        Rgb565::RED,
        Rgb565::BLUE,
        Rgb565::BLACK,
    ];
    let bar_w = W / bar_colors.len() as u32;
    let pixels = (0u32..(W * H)).map(|i| {
        let x = i % W;
        let y = i / W;
        if y < half {
            bar_colors[(x / bar_w).min(bar_colors.len() as u32 - 1) as usize]
        } else {
            let v = ((x as f32 / W as f32) * 31.0) as u8;
            Rgb565::new(v, v * 2, v)
        }
    });
    let area = Rectangle::new(Point::zero(), Size::new(W, H));
    display.fill_contiguous(&area, pixels).unwrap();
}

/// Checkerboard pattern
fn pattern_checkerboard(display: &mut Display) {
    let tile = 20u32;
    for ty in 0..(H / tile + 1) {
        for tx in 0..(W / tile + 1) {
            let color = if (tx + ty) % 2 == 0 {
                Rgb565::WHITE
            } else {
                Rgb565::BLACK
            };
            Rectangle::new(
                Point::new((tx * tile) as i32, (ty * tile) as i32),
                Size::new(tile, tile),
            )
            .into_styled(PrimitiveStyle::with_fill(color))
            .draw(display)
            .unwrap();
        }
    }
}

/// Concentric circles from center
fn pattern_circles(display: &mut Display) {
    fill(display, Rgb565::BLACK);
    let cx = W as i32 / 2;
    let cy = H as i32 / 2;
    let colors = [
        Rgb565::RED,
        Rgb565::CSS_ORANGE,
        Rgb565::CSS_YELLOW,
        Rgb565::GREEN,
        Rgb565::CYAN,
        Rgb565::BLUE,
        Rgb565::CSS_PURPLE,
    ];
    for (i, &color) in colors.iter().enumerate().rev() {
        let r = ((i as u32 + 1) * 12) as u32;
        Circle::new(Point::new(cx - r as i32, cy - r as i32), r * 2)
            .into_styled(PrimitiveStyle::with_fill(color))
            .draw(display)
            .unwrap();
    }
}

/// Grid / crosshatch pattern
fn pattern_grid(display: &mut Display) {
    fill(display, Rgb565::BLACK);
    let spacing = 20i32;
    let line_style = PrimitiveStyle::with_stroke(Rgb565::GREEN, 1);
    // Vertical lines
    let mut x = 0;
    while x < W as i32 {
        Rectangle::new(Point::new(x, 0), Size::new(1, H))
            .into_styled(line_style)
            .draw(display)
            .unwrap();
        x += spacing;
    }
    // Horizontal lines
    let mut y = 0;
    while y < H as i32 {
        Rectangle::new(Point::new(0, y), Size::new(W, 1))
            .into_styled(line_style)
            .draw(display)
            .unwrap();
        y += spacing;
    }
}
/// Pixel-spaced grid: white lines on black, `spacing` pixels apart
fn pattern_pixel_grid(display: &mut Display, spacing: i32) {
    let pixels = (0u32..(W * H)).map(|i| {
        let x = (i % W) as i32;
        let y = (i / W) as i32;
        if x % (spacing + 1) == 0 || y % (spacing + 1) == 0 {
            Rgb565::WHITE
        } else {
            Rgb565::BLACK
        }
    });
    let area = Rectangle::new(Point::zero(), Size::new(W, H));
    display.fill_contiguous(&area, pixels).unwrap();
}

/// Full-screen gray level fills, 10 steps from black to white with label
fn pattern_gray_levels(display: &mut Display) {
    const LABELS: [&str; 10] = [
        "0%", "11%", "22%", "33%", "44%", "56%", "67%", "78%", "89%", "100%",
    ];
    for (i, label) in LABELS.iter().enumerate() {
        let v = ((i as f32 / 9.0) * 31.0) as u8;
        let bg = Rgb565::new(v, v * 2, v);
        fill(display, bg);
        let text_color = if i >= 5 { Rgb565::BLACK } else { Rgb565::WHITE };
        let style = MonoTextStyle::new(&FONT_10X20, text_color);
        // Rough center: 10x20 font, label is short
        let x = (W as i32 - label.len() as i32 * 10) / 2;
        Text::new(label, Point::new(x, H as i32 / 2 + 5), style)
            .draw(display)
            .unwrap();
        embassy_time::block_for(Duration::from_millis(PAUSE_MS));
    }
}

/// Gray level bars: N discrete gray levels as vertical bars
fn pattern_gray_bars(display: &mut Display, levels: u32) {
    let bar_w = W / levels;
    for i in 0..levels {
        let v = ((i as f32 / (levels - 1) as f32) * 31.0) as u8;
        let color = Rgb565::new(v, v * 2, v);
        Rectangle::new(Point::new((i * bar_w) as i32, 0), Size::new(bar_w, H))
            .into_styled(PrimitiveStyle::with_fill(color))
            .draw(display)
            .unwrap();
    }
}

/// Gray ramp: stepped horizontal blocks (rows of gray levels)
fn pattern_gray_ramp(display: &mut Display) {
    let rows = 8u32;
    let cols = 16u32;
    let cell_w = W / cols;
    let cell_h = H / rows;
    for row in 0..rows {
        for col in 0..cols {
            let idx = row * cols + col;
            let total = rows * cols - 1;
            let v = ((idx as f32 / total as f32) * 31.0) as u8;
            let color = Rgb565::new(v, v * 2, v);
            Rectangle::new(
                Point::new((col * cell_w) as i32, (row * cell_h) as i32),
                Size::new(cell_w, cell_h),
            )
            .into_styled(PrimitiveStyle::with_fill(color))
            .draw(display)
            .unwrap();
        }
    }
}

/// Single-pixel checkerboard: alternating B/W pixels
fn pattern_pixel_checkerboard(display: &mut Display) {
    let pixels = (0u32..(W * H)).map(|i| {
        let x = i % W;
        let y = i / W;
        if (x + y) % 2 == 0 { Rgb565::WHITE } else { Rgb565::BLACK }
    });
    let area = Rectangle::new(Point::zero(), Size::new(W, H));
    display.fill_contiguous(&area, pixels).unwrap();
}

/// Per-channel gradient: full ramp for a single color channel
fn pattern_channel_gradient(display: &mut Display, channel: u8) {
    let pixels = (0u32..(W * H)).map(|i| {
        let x = i % W;
        let v = ((x as f32 / W as f32) * 31.0) as u8;
        match channel {
            0 => Rgb565::new(v, 0, 0),
            1 => Rgb565::new(0, v * 2, 0),
            _ => Rgb565::new(0, 0, v),
        }
    });
    let area = Rectangle::new(Point::zero(), Size::new(W, H));
    display.fill_contiguous(&area, pixels).unwrap();
}

/// Border test: 1px white border on black, verifies no edge clipping
fn pattern_border(display: &mut Display) {
    fill(display, Rgb565::BLACK);
    let s = PrimitiveStyle::with_stroke(Rgb565::WHITE, 1);
    // Top
    Rectangle::new(Point::zero(), Size::new(W, 1)).into_styled(s).draw(display).unwrap();
    // Bottom
    Rectangle::new(Point::new(0, H as i32 - 1), Size::new(W, 1)).into_styled(s).draw(display).unwrap();
    // Left
    Rectangle::new(Point::zero(), Size::new(1, H)).into_styled(s).draw(display).unwrap();
    // Right
    Rectangle::new(Point::new(W as i32 - 1, 0), Size::new(1, H)).into_styled(s).draw(display).unwrap();
    // Center label
    let style = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);
    Text::new("Border", Point::new(120, 90), style).draw(display).unwrap();
}

/// Crosshair at display center with tick marks
fn pattern_crosshair(display: &mut Display) {
    fill(display, Rgb565::BLACK);
    let cx = W as i32 / 2;
    let cy = H as i32 / 2;
    let s = PrimitiveStyle::with_stroke(Rgb565::WHITE, 1);
    // Horizontal line
    Line::new(Point::new(0, cy), Point::new(W as i32 - 1, cy)).into_styled(s).draw(display).unwrap();
    // Vertical line
    Line::new(Point::new(cx, 0), Point::new(cx, H as i32 - 1)).into_styled(s).draw(display).unwrap();
    // Tick marks every 20px on horizontal
    let tick = PrimitiveStyle::with_stroke(Rgb565::RED, 1);
    let mut x = 0;
    while x < W as i32 {
        Line::new(Point::new(x, cy - 3), Point::new(x, cy + 3)).into_styled(tick).draw(display).unwrap();
        x += 20;
    }
    // Tick marks every 20px on vertical
    let mut y = 0;
    while y < H as i32 {
        Line::new(Point::new(cx - 3, y), Point::new(cx + 3, y)).into_styled(tick).draw(display).unwrap();
        y += 20;
    }
    // Center dot
    Circle::new(Point::new(cx - 3, cy - 3), 6)
        .into_styled(PrimitiveStyle::with_fill(Rgb565::RED))
        .draw(display).unwrap();
}

/// Diagonal lines pattern
fn pattern_diagonals(display: &mut Display) {
    fill(display, Rgb565::BLACK);
    let s = PrimitiveStyle::with_stroke(Rgb565::WHITE, 1);
    // Main diagonals
    Line::new(Point::zero(), Point::new(W as i32 - 1, H as i32 - 1)).into_styled(s).draw(display).unwrap();
    Line::new(Point::new(W as i32 - 1, 0), Point::new(0, H as i32 - 1)).into_styled(s).draw(display).unwrap();
    // Parallel diagonals every 40px
    let s2 = PrimitiveStyle::with_stroke(Rgb565::CSS_DARK_GRAY, 1);
    let mut offset: i32 = 40;
    while offset < (W as i32 + H as i32) {
        Line::new(Point::new(offset, 0), Point::new(offset - H as i32, H as i32 - 1))
            .into_styled(s2).draw(display).unwrap();
        Line::new(Point::new(W as i32 - 1 - offset, 0), Point::new(W as i32 - 1 - offset + H as i32, H as i32 - 1))
            .into_styled(s2).draw(display).unwrap();
        offset += 40;
    }
}

/// Text readability: character map at two sizes
fn pattern_text_chart(display: &mut Display) {
    fill(display, Rgb565::BLACK);
    let big = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);
    let small = MonoTextStyle::new(&FONT_6X10, Rgb565::CSS_LIGHT_GRAY);
    Text::new("ABCDEFGHIJKLMNOPQRSTUVWXYZ", Point::new(5, 25), big).draw(display).unwrap();
    Text::new("abcdefghijklmnopqrstuvwxyz", Point::new(5, 50), big).draw(display).unwrap();
    Text::new("0123456789 !@#$%^&*()-+=", Point::new(5, 75), big).draw(display).unwrap();
    Text::new("ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz", Point::new(5, 100), small).draw(display).unwrap();
    Text::new("0123456789 !@#$%^&*()-+=[]{}|;':\",./<>?", Point::new(5, 115), small).draw(display).unwrap();
    Text::new("The quick brown fox jumps over the lazy dog", Point::new(5, 140), small).draw(display).unwrap();
    Text::new("THE QUICK BROWN FOX JUMPS OVER THE LAZY DOG", Point::new(5, 155), small).draw(display).unwrap();
}

/// Hue sweep: cycle through hues at full saturation
fn pattern_hue_sweep(display: &mut Display) {
    let pixels = (0u32..(W * H)).map(|i| {
        let x = i % W;
        // HSV to RGB, S=1, V=1, H = x mapped to 0..360
        let h = (x as f32 / W as f32) * 360.0;
        let c = 1.0_f32;
        let x2 = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
        let (r, g, b) = match (h as u32) / 60 {
            0 => (c, x2, 0.0),
            1 => (x2, c, 0.0),
            2 => (0.0, c, x2),
            3 => (0.0, x2, c),
            4 => (x2, 0.0, c),
            _ => (c, 0.0, x2),
        };
        Rgb565::new((r * 31.0) as u8, (g * 63.0) as u8, (b * 31.0) as u8)
    });
    let area = Rectangle::new(Point::zero(), Size::new(W, H));
    display.fill_contiguous(&area, pixels).unwrap();
}

/// Vertical gradient from black (top) to white (bottom)
fn pattern_vertical_gradient(display: &mut Display) {
    for y in 0..H {
        let v = ((y as f32 / H as f32) * 31.0) as u8;
        let color = Rgb565::new(v, v * 2, v);
        Rectangle::new(Point::new(0, y as i32), Size::new(W, 1))
            .into_styled(PrimitiveStyle::with_fill(color))
            .draw(display)
            .unwrap();
    }
}

/// Random-looking noise pattern (deterministic PRNG, no alloc)
fn pattern_noise(display: &mut Display) {
    let pixels = (0u32..(W * H)).map(|i| {
        // Simple xorshift-style hash
        let mut x = i.wrapping_mul(2654435761);
        x ^= x >> 16;
        x = x.wrapping_mul(0x45d9f3b);
        x ^= x >> 16;
        let v = (x & 0x1F) as u8;
        Rgb565::new(v, v * 2, v)
    });
    let area = Rectangle::new(Point::zero(), Size::new(W, H));
    display.fill_contiguous(&area, pixels).unwrap();
}

/// Horizontal stripes alternating colors
fn pattern_stripes(display: &mut Display) {
    let stripe_h = 10u32;
    let colors = [Rgb565::RED, Rgb565::WHITE];
    let mut y = 0u32;
    let mut i = 0usize;
    while y < H {
        let h = if y + stripe_h > H { H - y } else { stripe_h };
        Rectangle::new(Point::new(0, y as i32), Size::new(W, h))
            .into_styled(PrimitiveStyle::with_fill(colors[i % 2]))
            .draw(display)
            .unwrap();
        y += stripe_h;
        i += 1;
    }
}

#[embassy_executor::task]
async fn display_task(display: &'static mut Display<'static>, backlight: &'static mut Backlight) {
    info!("Display patterns task started");
    backlight.on();

    let pause = Duration::from_millis(PAUSE_MS);

    loop {
        info!("Solid colors");
        pattern_solid_colors(display);

        info!("Color bars");
        pattern_color_bars(display);
        Timer::after(pause).await;

        info!("Gradient");
        pattern_gradient(display);
        Timer::after(pause).await;

        info!("Split gradient");
        pattern_split_gradient(display);
        Timer::after(pause).await;

        info!("Gray bars 8");
        pattern_gray_bars(display, 8);
        Timer::after(pause).await;

        info!("Gray bars 16");
        pattern_gray_bars(display, 16);
        Timer::after(pause).await;

        info!("Gray bars 32");
        pattern_gray_bars(display, 32);
        Timer::after(pause).await;

        info!("Gray levels");
        pattern_gray_levels(display);

        info!("Gray ramp");
        pattern_gray_ramp(display);
        Timer::after(pause).await;

        info!("RGB gradient");
        pattern_rgb_gradient(display);
        Timer::after(pause).await;

        info!("Checkerboard");
        pattern_checkerboard(display);
        Timer::after(pause).await;

        info!("Circles");
        pattern_circles(display);
        Timer::after(pause).await;

        info!("Grid");
        pattern_grid(display);
        Timer::after(pause).await;

        info!("1px grid");
        pattern_pixel_grid(display, 1);
        Timer::after(pause).await;

        info!("2px grid");
        pattern_pixel_grid(display, 2);
        Timer::after(pause).await;

        info!("3px grid");
        pattern_pixel_grid(display, 3);
        Timer::after(pause).await;

        info!("4px grid");
        pattern_pixel_grid(display, 4);
        Timer::after(pause).await;

        info!("Stripes");
        pattern_stripes(display);
        Timer::after(pause).await;

        info!("Pixel checkerboard");
        pattern_pixel_checkerboard(display);
        Timer::after(pause).await;

        info!("Red channel gradient");
        pattern_channel_gradient(display, 0);
        Timer::after(pause).await;

        info!("Green channel gradient");
        pattern_channel_gradient(display, 1);
        Timer::after(pause).await;

        info!("Blue channel gradient");
        pattern_channel_gradient(display, 2);
        Timer::after(pause).await;

        info!("Border test");
        pattern_border(display);
        Timer::after(pause).await;

        info!("Crosshair");
        pattern_crosshair(display);
        Timer::after(pause).await;

        info!("Diagonals");
        pattern_diagonals(display);
        Timer::after(pause).await;

        info!("Text chart");
        pattern_text_chart(display);
        Timer::after(pause).await;

        info!("Hue sweep");
        pattern_hue_sweep(display);
        Timer::after(pause).await;

        info!("Vertical gradient");
        pattern_vertical_gradient(display);
        Timer::after(pause).await;

        info!("Noise");
        pattern_noise(display);
        Timer::after(pause).await;
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let peripherals = disobey2026badge::init();
    let resources = split_resources!(peripherals);

    esp_alloc::heap_allocator!(size: 128 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let display = mk_static!(Display<'static>, resources.display.into());
    let backlight = mk_static!(Backlight, resources.backlight.into());
    spawner.must_spawn(display_task(display, backlight));

    loop {
        Timer::after(Duration::from_secs(600)).await;
    }
}
