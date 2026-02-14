//! Renders a BMP image on the display.
//!
//! By default the image is centered. Press UP to align it to the top,
//! press DOWN to re-center. The image is drawn at its native resolution
//! (no resizing).
//!
//! Place your BMP file at `examples/assets/image.bmp`.
//! The image should be smaller than 320×170 to fit the screen.
//!
//! Convert a PNG to a compatible 24-bit BMP with ffmpeg:
//! ```sh
//! ffmpeg -y -i examples/assets/image.png -vf "scale=320:170:force_original_aspect_ratio=decrease,format=rgb24" -pix_fmt rgb24 -update 1 examples/assets/image.bmp
//! ```

#![no_std]
#![no_main]

use defmt::info;
#[allow(clippy::wildcard_imports)]
use disobey2026badge::*;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use embedded_graphics::{pixelcolor::{Rgb565, Rgb888}, prelude::*};
use esp_backtrace as _;
use esp_hal::timer::timg::TimerGroup;
use esp_println as _;
use tinybmp::Bmp;

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

const SCREEN_W: i32 = 320;
const SCREEN_H: i32 = 170;

/// Raw BMP data — replace with your own image.
const BMP_DATA: &[u8] = include_bytes!("assets/image.bmp");

#[embassy_executor::task]
async fn image_task(
    display: &'static mut Display<'static>,
    backlight: &'static mut Backlight,
    buttons: &'static mut Buttons,
) {
    backlight.on();

    let bmp: Bmp<Rgb888> = Bmp::from_slice(BMP_DATA).expect("Invalid BMP");
    let img_size = bmp.size();
    info!(
        "Image loaded: {}x{} px",
        img_size.width, img_size.height
    );

    let centered = Point::new(
        (SCREEN_W - img_size.width as i32) / 2,
        (SCREEN_H - img_size.height as i32) / 2,
    );
    let top = Point::new(
        (SCREEN_W - img_size.width as i32) / 2,
        0,
    );

    let mut position = centered;
    draw_image(display, &bmp, position);

    loop {
        let pressed = embassy_futures::select::select_array([
            Buttons::debounce_press(&mut buttons.up),
            Buttons::debounce_press(&mut buttons.down),
        ])
        .await;

        let new_pos = match pressed.1 {
            0 => {
                info!("Align: top");
                top
            }
            _ => {
                info!("Align: center");
                centered
            }
        };

        if new_pos != position {
            position = new_pos;
            draw_image(display, &bmp, position);
        }
    }
}

fn draw_image(display: &mut Display<'_>, bmp: &Bmp<Rgb888>, pos: Point) {
    // Clear screen
    display.clear(Rgb565::BLACK).unwrap();
    // Draw image, converting Rgb888 pixels to Rgb565
    let h = bmp.size().height as i32;
    let pixels = bmp.pixels().map(|Pixel(p, c)| {
        Pixel(
            Point::new(p.x, h - 1 - p.y) + pos,
            Rgb565::new(c.r() >> 3, c.g() >> 2, c.b() >> 3),
        )
    });
    display.draw_iter(pixels).unwrap();
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let peripherals = disobey2026badge::init();
    let resources = split_resources!(peripherals);

    esp_alloc::heap_allocator!(size: 200 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let display = mk_static!(Display<'static>, resources.display.into());
    let backlight = mk_static!(Backlight, resources.backlight.into());
    let buttons = mk_static!(Buttons, resources.buttons.into());

    spawner.must_spawn(image_task(display, backlight, buttons));

    loop {
        Timer::after(Duration::from_secs(600)).await;
    }
}
