//! Draws text and a colour gradient on the ST7789 display.

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
        iso_8859_1::FONT_10X20,
    },
    pixelcolor::Rgb565,
    prelude::*,
    primitives::Rectangle,
    text::Text,
};
use esp_backtrace as _;
use esp_hal::timer::timg::TimerGroup;
use esp_println as _;

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

#[embassy_executor::task]
async fn display_task(
    display: &'static mut disobey2026badge::Display<'static>,
    backlight: &'static mut Backlight,
) {
    info!("Display task started");

    // Draw a gradient background
    let gradient = (0u16..(320 * 170)).map(|i| {
        let x = i % 320;
        let y = i / 320;
        Rgb565::new(
            ((x as f32 / 320.0) * 31.0) as u8,
            0,
            ((y as f32 / 170.0) * 31.0) as u8,
        )
    });

    let area = Rectangle::new(Point::zero(), Size::new(320, 170));
    display.fill_contiguous(&area, gradient).unwrap();

    let style = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);
    Text::new("Disobey 2026 Badge", Point::new(40, 60), style)
        .draw(display)
        .unwrap();
    Text::new("disobey2026badge lib", Point::new(40, 90), style)
        .draw(display)
        .unwrap();

    info!("Display demo drawn — blinking backlight");

    loop {
        Timer::after(Duration::from_secs(3)).await;
        backlight.off();
        Timer::after(Duration::from_millis(200)).await;
        backlight.on();
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let peripherals = disobey2026badge::init();
    let resources = split_resources!(peripherals);

    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let display = mk_static!(disobey2026badge::Display<'static>, resources.display.into());
    let backlight = mk_static!(Backlight, resources.backlight.into());
    spawner.must_spawn(display_task(display, backlight));

    loop {
        Timer::after(Duration::from_secs(600)).await;
    }
}
