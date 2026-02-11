//! Two tasks take turns drawing on the display.
//!
//! Task A draws a bouncing ball, Task B draws a scrolling text banner.
//! A `Signal` acts as a baton — whichever task holds it draws for a while,
//! then signals the other to take over.

#![no_std]
#![no_main]

use defmt::info;
#[allow(clippy::wildcard_imports)]
use disobey2026badge::*;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use embedded_graphics::{
    mono_font::{MonoTextStyle, iso_8859_1::FONT_10X20},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{Circle, PrimitiveStyle, Rectangle},
    text::Text,
};
use esp_backtrace as _;
use esp_hal::timer::timg::TimerGroup;
use esp_println as _;

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

const W: i32 = 320;
const H: i32 = 170;

/// Which task should be active.
#[derive(Clone, Copy, defmt::Format)]
enum Turn {
    Ball,
    Banner,
}

/// Shared signal used as a baton between the two tasks.
static TURN: Signal<CriticalSectionRawMutex, Turn> = Signal::new();

fn clear(display: &mut Display, color: Rgb565) {
    Rectangle::new(Point::zero(), Size::new(W as u32, H as u32))
        .into_styled(PrimitiveStyle::with_fill(color))
        .draw(display)
        .unwrap();
}

/// Task A: bouncing ball on a dark blue background.
#[embassy_executor::task]
async fn ball_task(display: &'static mut Display<'static>) {
    let mut x: i32 = 40;
    let mut y: i32 = 85;
    let mut dx: i32 = 3;
    let mut dy: i32 = 2;
    let r: i32 = 12;

    loop {
        // Wait for our turn
        loop {
            let turn = TURN.wait().await;
            if matches!(turn, Turn::Ball) {
                break;
            }
            // Not our turn — re-signal so the other task sees it
            TURN.signal(turn);
            Timer::after(Duration::from_millis(10)).await;
        }

        info!("Ball task: my turn");
        let label = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);

        // Animate for ~3 seconds (60 frames at 50ms)
        for _ in 0..60 {
            clear(display, Rgb565::new(0, 0, 8));
            Text::new("BALL", Point::new(5, 20), label)
                .draw(display)
                .unwrap();

            // Move
            x += dx;
            y += dy;
            if x - r <= 0 || x + r >= W {
                dx = -dx;
            }
            if y - r <= 0 || y + r >= H {
                dy = -dy;
            }
            x = x.clamp(r, W - r);
            y = y.clamp(r, H - r);

            Circle::new(Point::new(x - r, y - r), (r * 2) as u32)
                .into_styled(PrimitiveStyle::with_fill(Rgb565::CSS_ORANGE))
                .draw(display)
                .unwrap();

            Timer::after(Duration::from_millis(50)).await;
        }

        info!("Ball task: handing off to banner");
        TURN.signal(Turn::Banner);
    }
}

/// Task B: scrolling text banner on a dark green background.
#[embassy_executor::task]
async fn banner_task(display: &'static mut Display<'static>) {
    let mut offset: i32 = W;

    loop {
        // Wait for our turn
        loop {
            let turn = TURN.wait().await;
            if matches!(turn, Turn::Banner) {
                break;
            }
            TURN.signal(turn);
            Timer::after(Duration::from_millis(10)).await;
        }

        info!("Banner task: my turn");
        let style = MonoTextStyle::new(&FONT_10X20, Rgb565::CSS_YELLOW);
        let label = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);
        let msg = "** DISOBEY 2026 **";

        // Scroll for ~3 seconds (60 frames at 50ms)
        for _ in 0..60 {
            clear(display, Rgb565::new(0, 8, 0));
            Text::new("BANNER", Point::new(5, 20), label)
                .draw(display)
                .unwrap();

            Text::new(msg, Point::new(offset, H / 2 + 5), style)
                .draw(display)
                .unwrap();

            offset -= 4;
            if offset < -(msg.len() as i32 * 10) {
                offset = W;
            }

            Timer::after(Duration::from_millis(50)).await;
        }

        info!("Banner task: handing off to ball");
        TURN.signal(Turn::Ball);
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let peripherals = disobey2026badge::init();
    let resources = split_resources!(peripherals);

    esp_alloc::heap_allocator!(size: 128 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let display: Display<'static> = resources.display.into();
    let backlight = mk_static!(Backlight, resources.backlight.into());
    backlight.on();

    // Both tasks need &'static mut Display, but there's only one display.
    // We use the signal to ensure only one task draws at a time.
    // Split into two pointers — safety relies on the signal protocol.
    let display_ptr = mk_static!(Display<'static>, display) as *mut Display<'static>;

    let display_a: &'static mut Display<'static> = unsafe { &mut *display_ptr };
    let display_b: &'static mut Display<'static> = unsafe { &mut *display_ptr };

    spawner.must_spawn(ball_task(display_a));
    spawner.must_spawn(banner_task(display_b));

    // Kick things off — ball goes first
    TURN.signal(Turn::Ball);

    loop {
        Timer::after(Duration::from_secs(600)).await;
    }
}
