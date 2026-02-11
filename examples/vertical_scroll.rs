//! Hardware vertical scrolling demo using the ST7789's built-in VSCRDEF/VSCRSADD commands.
//!
//! Draws colored stripes across the display, then uses the ST7789's hardware
//! vertical scroll feature to smoothly scroll them without redrawing.
//!
//! Note: ST7789 vertical scrolling operates on the framebuffer's native
//! vertical axis (320 px). With the badge's Deg90 rotation this appears as
//! horizontal movement on screen.

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

/// Stripe colors for the scrolling region.
const COLORS: &[Rgb565] = &[
    Rgb565::RED,
    Rgb565::GREEN,
    Rgb565::BLUE,
    Rgb565::YELLOW,
    Rgb565::CYAN,
    Rgb565::CSS_ORANGE,
    Rgb565::MAGENTA,
    Rgb565::WHITE,
];

#[embassy_executor::task]
async fn scroll_task(
    display: &'static mut disobey2026badge::Display<'static>,
    backlight: &'static mut Backlight,
) {
    backlight.on();
    info!("Vertical scroll demo started");

    // Draw colored stripes — each stripe is 320 / COLORS.len() = 40 pixels wide.
    // In the rotated view these appear as vertical bands across the screen.
    let stripe_w = 320u32 / COLORS.len() as u32;
    for (i, &color) in COLORS.iter().enumerate() {
        let x = i as i32 * stripe_w as i32;
        Rectangle::new(Point::new(x, 0), Size::new(stripe_w, 170))
            .into_styled(
                embedded_graphics::primitives::PrimitiveStyle::with_fill(color),
            )
            .draw(display)
            .unwrap();
    }

    info!("Stripes drawn, starting hardware scroll");

    // Set up the scroll region: no fixed areas, entire framebuffer scrolls.
    // The ST7789 framebuffer height (default orientation) is 320.
    display.set_vertical_scroll_region(0, 0).unwrap();

    // Scroll continuously — the offset wraps around at 320.
    let mut offset: u16 = 0;
    loop {
        display.set_vertical_scroll_offset(offset).unwrap();
        offset = offset.wrapping_add(1) % 320;
        Timer::after(Duration::from_millis(10)).await; // ~60 fps
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let peripherals = disobey2026badge::init();
    let resources = split_resources!(peripherals);

//    esp_alloc::heap_allocator!(size: 128 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let display = mk_static!(disobey2026badge::Display<'static>, resources.display.into());
    let backlight = mk_static!(Backlight, resources.backlight.into());
    spawner.must_spawn(scroll_task(display, backlight));

    loop {
        Timer::after(Duration::from_secs(600)).await;
    }
}
