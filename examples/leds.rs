//! Cycles a rainbow across the 10 WS2812 LEDs.

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
use esp_backtrace as _;
use esp_hal::timer::timg::TimerGroup;
use esp_println as _;
use palette::Srgb;

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

#[embassy_executor::task]
async fn led_task(leds: &'static mut Leds<'static>) {
    info!("LED task started — rainbow cycle");

    let colors: [Srgb<u8>; 10] = [
        Srgb::new(20, 0, 0),
        Srgb::new(20, 10, 0),
        Srgb::new(20, 20, 0),
        Srgb::new(0, 20, 0),
        Srgb::new(0, 20, 10),
        Srgb::new(0, 20, 20),
        Srgb::new(0, 0, 20),
        Srgb::new(10, 0, 20),
        Srgb::new(20, 0, 20),
        Srgb::new(20, 0, 10),
    ];

    let mut offset = 0usize;
    loop {
        for i in 0..leds.len() {
            leds.set(i, colors[(i + offset) % colors.len()]);
        }
        leds.update().await;

        offset = (offset + 1) % colors.len();
        Timer::after(Duration::from_millis(100)).await;
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let peripherals = disobey2026badge::init();
    let resources = split_resources!(peripherals);

    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let leds = mk_static!(Leds<'static>, resources.leds.into());
    spawner.must_spawn(led_task(leds));

    loop {
        Timer::after(Duration::from_secs(600)).await;
    }
}
