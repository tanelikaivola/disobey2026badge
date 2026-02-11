//! Pulses the vibration motor in a heartbeat pattern.

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
async fn vibration_task(motor: &'static mut Vibration) {
    info!("Vibration task started — heartbeat pattern");

    loop {
        motor.pulse(Duration::from_millis(80)).await;
        Timer::after(Duration::from_millis(120)).await;
        motor.pulse(Duration::from_millis(80)).await;
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

    let motor = mk_static!(Vibration, resources.vibra.into());
    spawner.must_spawn(vibration_task(motor));

    loop {
        Timer::after(Duration::from_secs(600)).await;
    }
}
