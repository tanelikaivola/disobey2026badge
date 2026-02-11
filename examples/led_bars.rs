//! Demonstrates the left/right LED bar functions.
//!
//! 1. Sets both bars to the same gradient — they should look symmetrical.
//! 2. Sets each bar independently with different colors.
//! 3. Scrolls a single lit LED up both bars in sync.

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

const OFF: Srgb<u8> = Srgb::new(0, 0, 0);

#[embassy_executor::task]
async fn led_task(leds: &'static mut Leds<'static>) {
    // A bottom-to-top green gradient used for both bars.
    let gradient: [Srgb<u8>; BAR_COUNT] = [
        Srgb::new(0, 4, 0),
        Srgb::new(0, 8, 0),
        Srgb::new(0, 14, 0),
        Srgb::new(0, 20, 0),
        Srgb::new(0, 28, 0),
    ];

    loop {
        // ── Phase 1: both bars identical (symmetrical) ──────────────────
        info!("Phase 1: both bars — green gradient");
        leds.set_both_bars(&gradient);
        leds.update().await;
        Timer::after(Duration::from_secs(2)).await;

        // ── Phase 2: left red, right blue ───────────────────────────────
        info!("Phase 2: left red, right blue");
        let red: [Srgb<u8>; BAR_COUNT] = [Srgb::new(20, 0, 0); BAR_COUNT];
        let blue: [Srgb<u8>; BAR_COUNT] = [Srgb::new(0, 0, 20); BAR_COUNT];
        leds.set_left_bar(&red);
        leds.set_right_bar(&blue);
        leds.update().await;
        Timer::after(Duration::from_secs(2)).await;

        // ── Phase 3: scrolling dot up both bars ─────────────────────────
        info!("Phase 3: scrolling dot");
        for _ in 0..5 {
            for i in 0..BAR_COUNT {
                let mut bar = [OFF; BAR_COUNT];
                bar[i] = Srgb::new(20, 20, 20);
                leds.set_both_bars(&bar);
                leds.update().await;
                Timer::after(Duration::from_millis(150)).await;
            }
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

    let leds = mk_static!(Leds<'static>, resources.leds.into());
    spawner.must_spawn(led_task(leds));

    loop {
        Timer::after(Duration::from_secs(600)).await;
    }
}
