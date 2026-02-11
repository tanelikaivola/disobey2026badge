//! Prints which button was pressed via defmt logging.

#![no_std]
#![no_main]

use defmt::info;
#[allow(clippy::wildcard_imports)]
use disobey2026badge::*;
use embassy_executor::Spawner;
use esp_backtrace as _;
use esp_hal::timer::timg::TimerGroup;
use esp_println as _;

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

#[embassy_executor::task]
async fn button_task(buttons: &'static mut Buttons) {
    info!("Button task started — press any button");

    loop {
        let pressed = embassy_futures::select::select_array([
            Buttons::debounce_press(&mut buttons.up),
            Buttons::debounce_press(&mut buttons.down),
            Buttons::debounce_press(&mut buttons.left),
            Buttons::debounce_press(&mut buttons.right),
            Buttons::debounce_press(&mut buttons.stick),
            Buttons::debounce_press(&mut buttons.a),
            Buttons::debounce_press(&mut buttons.b),
            Buttons::debounce_press(&mut buttons.start),
            Buttons::debounce_press(&mut buttons.select),
        ])
        .await;

        let name = match pressed.1 {
            0 => "UP",
            1 => "DOWN",
            2 => "LEFT",
            3 => "RIGHT",
            4 => "STICK",
            5 => "A",
            6 => "B",
            7 => "START",
            8 => "SELECT",
            _ => "???",
        };

        info!("Button pressed: {}", name);
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let peripherals = disobey2026badge::init();
    let resources = split_resources!(peripherals);

    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let buttons = mk_static!(Buttons, resources.buttons.into());
    spawner.must_spawn(button_task(buttons));

    loop {
        embassy_time::Timer::after(embassy_time::Duration::from_secs(600)).await;
    }
}
