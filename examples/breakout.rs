//! Breakout game for the Disobey 2026 badge.
//!
//! - Left/Right buttons move the paddle
//! - Ball bounces off walls, paddle, and bricks
//! - LEDs flash when a brick is destroyed
//! - Press A to launch the ball / restart after game over

#![no_std]
#![no_main]

use defmt::info;
#[allow(clippy::wildcard_imports)]
use disobey2026badge::*;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use embedded_graphics::{
    mono_font::{MonoTextStyle, iso_8859_1::FONT_6X10},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::Text,
};
use esp_backtrace as _;
use esp_hal::timer::timg::TimerGroup;
use esp_println as _;
use palette::Srgb;

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

// Display dimensions
const W: i32 = 320;
const H: i32 = 170;

// Paddle
const PADDLE_W: i32 = 40;
const PADDLE_H: i32 = 6;
const PADDLE_Y: i32 = H - 12;
const PADDLE_SPEED: i32 = 6;

// Ball
const BALL_SIZE: i32 = 4;

// Bricks
const BRICK_COLS: usize = 10;
const BRICK_ROWS: usize = 4;
const BRICK_W: i32 = 28;
const BRICK_H: i32 = 10;
const BRICK_GAP: i32 = 2;
const BRICK_OFFSET_X: i32 = (W - (BRICK_W + BRICK_GAP) * BRICK_COLS as i32 + BRICK_GAP) / 2;
const BRICK_OFFSET_Y: i32 = 20;

// LED flash duration in game ticks
const LED_FLASH_TICKS: u8 = 6;

// Game tick rate
const TICK_MS: u64 = 20;

const BRICK_COLORS: [Rgb565; BRICK_ROWS] = [
    Rgb565::RED,
    Rgb565::CSS_ORANGE,
    Rgb565::CSS_YELLOW,
    Rgb565::GREEN,
];

struct Game {
    paddle_x: i32,
    ball_x: i32,
    ball_y: i32,
    ball_dx: i32,
    ball_dy: i32,
    bricks: [[bool; BRICK_COLS]; BRICK_ROWS],
    score: u16,
    lives: u8,
    launched: bool,
    game_over: bool,
    led_flash: u8,
}

impl Game {
    fn new() -> Self {
        Self {
            paddle_x: W / 2 - PADDLE_W / 2,
            ball_x: W / 2,
            ball_y: PADDLE_Y - BALL_SIZE - 1,
            ball_dx: 2,
            ball_dy: -2,
            bricks: [[true; BRICK_COLS]; BRICK_ROWS],
            score: 0,
            lives: 3,
            launched: false,
            game_over: false,
            led_flash: 0,
        }
    }

    fn reset_ball(&mut self) {
        self.ball_x = self.paddle_x + PADDLE_W / 2;
        self.ball_y = PADDLE_Y - BALL_SIZE - 1;
        self.ball_dx = 2;
        self.ball_dy = -2;
        self.launched = false;
    }

    fn bricks_remaining(&self) -> u16 {
        let mut count = 0u16;
        for row in &self.bricks {
            for &b in row {
                if b {
                    count += 1;
                }
            }
        }
        count
    }

    fn tick(&mut self) {
        if self.game_over || !self.launched {
            return;
        }

        if self.led_flash > 0 {
            self.led_flash -= 1;
        }

        // Move ball
        self.ball_x += self.ball_dx;
        self.ball_y += self.ball_dy;

        // Wall collisions
        if self.ball_x <= 0 {
            self.ball_x = 0;
            self.ball_dx = self.ball_dx.abs();
        }
        if self.ball_x + BALL_SIZE >= W {
            self.ball_x = W - BALL_SIZE;
            self.ball_dx = -self.ball_dx.abs();
        }
        if self.ball_y <= 0 {
            self.ball_y = 0;
            self.ball_dy = self.ball_dy.abs();
        }

        // Ball fell below paddle
        if self.ball_y + BALL_SIZE >= H {
            self.lives = self.lives.saturating_sub(1);
            if self.lives == 0 {
                self.game_over = true;
            } else {
                self.reset_ball();
            }
            return;
        }

        // Paddle collision
        if self.ball_dy > 0
            && self.ball_y + BALL_SIZE >= PADDLE_Y
            && self.ball_y + BALL_SIZE <= PADDLE_Y + PADDLE_H
            && self.ball_x + BALL_SIZE > self.paddle_x
            && self.ball_x < self.paddle_x + PADDLE_W
        {
            self.ball_dy = -self.ball_dy.abs();
            // Angle based on where ball hits paddle
            let hit_pos = self.ball_x + BALL_SIZE / 2 - self.paddle_x;
            let third = PADDLE_W / 3;
            if hit_pos < third {
                self.ball_dx = -3;
            } else if hit_pos > third * 2 {
                self.ball_dx = 3;
            } else {
                // Keep current dx direction but normalize speed
                self.ball_dx = if self.ball_dx > 0 { 2 } else { -2 };
            }
        }

        // Brick collisions
        for row in 0..BRICK_ROWS {
            for col in 0..BRICK_COLS {
                if !self.bricks[row][col] {
                    continue;
                }
                let bx = BRICK_OFFSET_X + col as i32 * (BRICK_W + BRICK_GAP);
                let by = BRICK_OFFSET_Y + row as i32 * (BRICK_H + BRICK_GAP);

                if self.ball_x + BALL_SIZE > bx
                    && self.ball_x < bx + BRICK_W
                    && self.ball_y + BALL_SIZE > by
                    && self.ball_y < by + BRICK_H
                {
                    self.bricks[row][col] = false;
                    self.score += (BRICK_ROWS - row) as u16;
                    self.led_flash = LED_FLASH_TICKS;

                    // Determine bounce direction
                    let ball_cx = self.ball_x + BALL_SIZE / 2;
                    let ball_cy = self.ball_y + BALL_SIZE / 2;
                    let brick_cx = bx + BRICK_W / 2;
                    let brick_cy = by + BRICK_H / 2;

                    let dx = (ball_cx - brick_cx).abs() * BRICK_H;
                    let dy = (ball_cy - brick_cy).abs() * BRICK_W;

                    if dx > dy {
                        self.ball_dx = -self.ball_dx;
                    } else {
                        self.ball_dy = -self.ball_dy;
                    }

                    // Win check
                    if self.bricks_remaining() == 0 {
                        self.game_over = true;
                    }
                    return; // Only destroy one brick per tick
                }
            }
        }
    }
}

/// Tracks previous frame positions so we only erase what moved.
struct PrevState {
    ball_x: i32,
    ball_y: i32,
    paddle_x: i32,
    score: u16,
    lives: u8,
    bricks: [[bool; BRICK_COLS]; BRICK_ROWS],
}

const BLACK: PrimitiveStyle<Rgb565> = PrimitiveStyle::with_fill(Rgb565::BLACK);
const WHITE: PrimitiveStyle<Rgb565> = PrimitiveStyle::with_fill(Rgb565::WHITE);

/// Draw the full initial game screen (once per round).
fn draw_initial(display: &mut Display, game: &Game) {
    // Clear once
    Rectangle::new(Point::zero(), Size::new(W as u32, H as u32))
        .into_styled(BLACK)
        .draw(display)
        .unwrap();

    // All bricks
    for row in 0..BRICK_ROWS {
        for col in 0..BRICK_COLS {
            let x = BRICK_OFFSET_X + col as i32 * (BRICK_W + BRICK_GAP);
            let y = BRICK_OFFSET_Y + row as i32 * (BRICK_H + BRICK_GAP);
            Rectangle::new(Point::new(x, y), Size::new(BRICK_W as u32, BRICK_H as u32))
                .into_styled(PrimitiveStyle::with_fill(BRICK_COLORS[row]))
                .draw(display)
                .unwrap();
        }
    }

    // Paddle
    Rectangle::new(
        Point::new(game.paddle_x, PADDLE_Y),
        Size::new(PADDLE_W as u32, PADDLE_H as u32),
    )
    .into_styled(WHITE)
    .draw(display)
    .unwrap();

    // Ball
    Rectangle::new(
        Point::new(game.ball_x, game.ball_y),
        Size::new(BALL_SIZE as u32, BALL_SIZE as u32),
    )
    .into_styled(WHITE)
    .draw(display)
    .unwrap();

    // HUD
    draw_hud(display, game.score, game.lives);
}

fn draw_hud(display: &mut Display, score: u16, lives: u8) {
    // Clear HUD area
    Rectangle::new(Point::zero(), Size::new(W as u32, 14))
        .into_styled(BLACK)
        .draw(display)
        .unwrap();

    let style = MonoTextStyle::new(&FONT_6X10, Rgb565::WHITE);
    let mut buf = [0u8; 16];
    let score_str = format_u16(score, &mut buf);
    Text::new(score_str, Point::new(4, 10), style)
        .draw(display)
        .unwrap();

    for i in 0..lives {
        Rectangle::new(
            Point::new(W - 12 - i as i32 * 10, 2),
            Size::new(6, 6),
        )
        .into_styled(PrimitiveStyle::with_fill(Rgb565::RED))
        .draw(display)
        .unwrap();
    }
}

/// Incremental frame draw — only erases and redraws what changed.
fn draw_frame(display: &mut Display, game: &Game, prev: &PrevState) {
    // Erase old ball
    Rectangle::new(
        Point::new(prev.ball_x, prev.ball_y),
        Size::new(BALL_SIZE as u32, BALL_SIZE as u32),
    )
    .into_styled(BLACK)
    .draw(display)
    .unwrap();

    // Erase old paddle (only the parts that aren't covered by new position)
    if prev.paddle_x != game.paddle_x {
        Rectangle::new(
            Point::new(prev.paddle_x, PADDLE_Y),
            Size::new(PADDLE_W as u32, PADDLE_H as u32),
        )
        .into_styled(BLACK)
        .draw(display)
        .unwrap();
    }

    // Black out any newly destroyed bricks
    for row in 0..BRICK_ROWS {
        for col in 0..BRICK_COLS {
            if prev.bricks[row][col] && !game.bricks[row][col] {
                let x = BRICK_OFFSET_X + col as i32 * (BRICK_W + BRICK_GAP);
                let y = BRICK_OFFSET_Y + row as i32 * (BRICK_H + BRICK_GAP);
                Rectangle::new(Point::new(x, y), Size::new(BRICK_W as u32, BRICK_H as u32))
                    .into_styled(BLACK)
                    .draw(display)
                    .unwrap();
            }
        }
    }

    // Draw paddle at new position
    Rectangle::new(
        Point::new(game.paddle_x, PADDLE_Y),
        Size::new(PADDLE_W as u32, PADDLE_H as u32),
    )
    .into_styled(WHITE)
    .draw(display)
    .unwrap();

    // Draw ball at new position
    Rectangle::new(
        Point::new(game.ball_x, game.ball_y),
        Size::new(BALL_SIZE as u32, BALL_SIZE as u32),
    )
    .into_styled(WHITE)
    .draw(display)
    .unwrap();

    // Redraw HUD — ball can pass through the HUD area and erase it
    if prev.ball_y < 14 || prev.score != game.score || prev.lives != game.lives {
        draw_hud(display, game.score, game.lives);
    }
}

fn draw_title(display: &mut Display) {
    Rectangle::new(Point::zero(), Size::new(W as u32, H as u32))
        .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
        .draw(display)
        .unwrap();

    let big = MonoTextStyle::new(&FONT_6X10, Rgb565::CSS_YELLOW);
    let small = MonoTextStyle::new(&FONT_6X10, Rgb565::WHITE);

    Text::new("BREAKOUT", Point::new(W / 2 - 24, H / 2 - 10), big)
        .draw(display)
        .unwrap();
    Text::new("Press A to start", Point::new(W / 2 - 48, H / 2 + 10), small)
        .draw(display)
        .unwrap();
}

fn draw_game_over(display: &mut Display, won: bool, score: u16) {
    Rectangle::new(Point::zero(), Size::new(W as u32, H as u32))
        .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
        .draw(display)
        .unwrap();

    let color = if won { Rgb565::GREEN } else { Rgb565::RED };
    let msg = if won { "YOU WIN!" } else { "GAME OVER" };
    let style = MonoTextStyle::new(&FONT_6X10, color);
    let small = MonoTextStyle::new(&FONT_6X10, Rgb565::WHITE);

    Text::new(msg, Point::new(W / 2 - 30, H / 2 - 10), style)
        .draw(display)
        .unwrap();

    let mut buf = [0u8; 24];
    let score_str = format_score(score, &mut buf);
    Text::new(score_str, Point::new(W / 2 - 30, H / 2 + 5), small)
        .draw(display)
        .unwrap();

    Text::new("Press A to restart", Point::new(W / 2 - 54, H / 2 + 20), small)
        .draw(display)
        .unwrap();
}

/// Format a u16 into a string buffer, returns the slice.
fn format_u16(mut n: u16, buf: &mut [u8; 16]) -> &str {
    if n == 0 {
        buf[0] = b'0';
        return unsafe { core::str::from_utf8_unchecked(&buf[..1]) };
    }
    let mut i = 0;
    let mut tmp = [0u8; 5];
    while n > 0 {
        tmp[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    for j in 0..i {
        buf[j] = tmp[i - 1 - j];
    }
    unsafe { core::str::from_utf8_unchecked(&buf[..i]) }
}

/// Format "Score: NNN" into a buffer.
fn format_score(score: u16, buf: &mut [u8; 24]) -> &str {
    let prefix = b"Score: ";
    buf[..prefix.len()].copy_from_slice(prefix);
    let mut num_buf = [0u8; 16];
    let num_str = format_u16(score, &mut num_buf);
    let num_bytes = num_str.as_bytes();
    buf[prefix.len()..prefix.len() + num_bytes.len()].copy_from_slice(num_bytes);
    let total = prefix.len() + num_bytes.len();
    unsafe { core::str::from_utf8_unchecked(&buf[..total]) }
}

fn update_leds(leds: &mut Leds, game: &Game) {
    if game.led_flash > 0 {
        // Flash bright white on hit
        let brightness = (game.led_flash as u8) * 4;
        let color = Srgb::new(brightness, brightness, brightness);
        leds.fill(color);
    } else {
        // Show remaining bricks as a bar graph on LEDs
        let remaining = game.bricks_remaining();
        let total = (BRICK_ROWS * BRICK_COLS) as u16;
        let lit = ((remaining as u32 * 5 + total as u32 - 1) / total as u32) as usize;

        let mut left = [Srgb::new(0u8, 0, 0); BAR_COUNT];
        let mut right = [Srgb::new(0u8, 0, 0); BAR_COUNT];
        for i in 0..lit.min(BAR_COUNT) {
            let color = Srgb::new(0, 4, 2);
            left[i] = color;
            right[i] = color;
        }
        leds.set_left_bar(&left);
        leds.set_right_bar(&right);
    }
}

#[embassy_executor::task]
async fn game_task(
    display: &'static mut Display<'static>,
    backlight: &'static mut Backlight,
    leds: &'static mut Leds<'static>,
    buttons: &'static mut Buttons,
) {
    info!("Breakout game task started");
    backlight.on();

    loop {
        // Title screen
        draw_title(display);
        leds.clear();
        leds.update().await;

        // Wait for A press
        Buttons::debounce_press(&mut buttons.a).await;

        // Game loop
        let mut game = Game::new();
        draw_initial(display, &game);
        let mut prev = PrevState {
            ball_x: game.ball_x,
            ball_y: game.ball_y,
            paddle_x: game.paddle_x,
            score: game.score,
            lives: game.lives,
            bricks: game.bricks,
        };
        let tick = Duration::from_millis(TICK_MS);

        loop {
            // Poll held buttons directly each tick
            if buttons.left.is_low() {
                game.paddle_x = (game.paddle_x - PADDLE_SPEED).max(0);
                if !game.launched {
                    game.ball_x = game.paddle_x + PADDLE_W / 2;
                }
            }
            if buttons.right.is_low() {
                game.paddle_x = (game.paddle_x + PADDLE_SPEED).min(W - PADDLE_W);
                if !game.launched {
                    game.ball_x = game.paddle_x + PADDLE_W / 2;
                }
            }

            // Check A for launch
            if !game.launched && buttons.a.is_low() {
                game.launched = true;
            }

            game.tick();

            draw_frame(display, &game, &prev);
            prev.ball_x = game.ball_x;
            prev.ball_y = game.ball_y;
            prev.paddle_x = game.paddle_x;
            prev.score = game.score;
            prev.lives = game.lives;
            prev.bricks = game.bricks;

            update_leds(leds, &game);
            leds.update().await;

            if game.game_over {
                let won = game.bricks_remaining() == 0;
                Timer::after(Duration::from_millis(500)).await;
                draw_game_over(display, won, game.score);

                // Flash LEDs for game over
                if won {
                    for _ in 0..3 {
                        leds.fill(Srgb::new(0, 20, 0));
                        leds.update().await;
                        Timer::after(Duration::from_millis(300)).await;
                        leds.clear();
                        leds.update().await;
                        Timer::after(Duration::from_millis(300)).await;
                    }
                } else {
                    for _ in 0..3 {
                        leds.fill(Srgb::new(20, 0, 0));
                        leds.update().await;
                        Timer::after(Duration::from_millis(300)).await;
                        leds.clear();
                        leds.update().await;
                        Timer::after(Duration::from_millis(300)).await;
                    }
                }

                // Wait for restart
                Buttons::debounce_press(&mut buttons.a).await;
                break; // Restart outer loop
            }

            Timer::after(tick).await;
        }
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let peripherals = disobey2026badge::init();
    let resources = split_resources!(peripherals);

    esp_alloc::heap_allocator!(size: 128 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let display = mk_static!(Display<'static>, resources.display.into());
    let backlight = mk_static!(Backlight, resources.backlight.into());
    let leds = mk_static!(Leds<'static>, resources.leds.into());
    let buttons = mk_static!(Buttons, resources.buttons.into());

    spawner.must_spawn(game_task(display, backlight, leds, buttons));

    loop {
        Timer::after(Duration::from_secs(600)).await;
    }
}
