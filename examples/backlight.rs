//! Toggles the display backlight on and off every second.

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

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

#[embassy_executor::task]
async fn backlight_task(backlight: &'static mut Backlight) {
    info!("Backlight task started — toggling every second");

    loop {
        backlight.toggle();
        info!(
            "Backlight: {}",
            if backlight.is_on() { "ON" } else { "OFF" }
        );
        Timer::after(Duration::from_secs(1)).await;
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let peripherals = disobey2026badge::init();
    let resources = split_resources!(peripherals);

    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let backlight = mk_static!(Backlight, resources.backlight.into());
    spawner.must_spawn(backlight_task(backlight));

    loop {
        Timer::after(Duration::from_secs(600)).await;
    }
}
