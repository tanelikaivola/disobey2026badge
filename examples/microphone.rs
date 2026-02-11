//! Reads audio samples from the I2S microphone and logs the peak amplitude.

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
use esp_hal::{
    dma::DmaDescriptor,
    timer::timg::TimerGroup,
};
use esp_println as _;

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

#[embassy_executor::task]
async fn mic_task(mic: &'static mut microphone::Microphone<'static>) {
    info!("Microphone task started — reading samples");

    let mut buf = [0i16; 1024];
    loop {
        match mic.rx.read_words(&mut buf) {
            Ok(()) => {
                // Find peak amplitude in this batch
                let peak = buf.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
                info!("Peak amplitude: {}", peak);
            }
            Err(e) => {
                info!("Read error: {}", e);
            }
        }

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

    let descriptors = mk_static!([DmaDescriptor; 8], [DmaDescriptor::EMPTY; 8]);

    let mic = mk_static!(
        microphone::Microphone<'static>,
        microphone::Microphone::new(resources.mic, microphone::DEFAULT_SAMPLE_RATE, descriptors,)
    );

    spawner.must_spawn(mic_task(mic));

    loop {
        Timer::after(Duration::from_secs(600)).await;
    }
}
