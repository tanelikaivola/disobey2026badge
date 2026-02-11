//! WS2812 addressable LED driver using the RMT peripheral.
//!
//! The badge has 10 RGB LEDs arranged in a strip.

extern crate alloc;

use defmt::error;
use embassy_time::{
    Duration,
    Timer,
};
use esp_hal::{
    Blocking,
    gpio::Level,
    rmt::{
        PulseCode,
        Tx,
    },
};
use palette::Srgb;

/// Number of WS2812 LEDs on the badge.
/// There are two led bars with 5 leds each. Left and right. Indexing is counter clockwise starting from the bottom right.
/// Index 0 is bottom right. Index 4 is top right. Index 5 is top left. Index 9 is bottom left.
pub const LED_COUNT: usize = 10;

/// Number of LEDs per bar (left or right).
pub const BAR_COUNT: usize = 5;

/// WS2812 LED strip driver.
///
/// Maintains an in-memory framebuffer that is flushed to hardware
/// with [`update`](Leds::update).
pub struct Leds<'a> {
    channel: Option<esp_hal::rmt::Channel<'a, Blocking, Tx>>,
    framebuffer: [Srgb<u8>; LED_COUNT],
}

impl<'a> Leds<'a> {
    pub const fn new(channel: esp_hal::rmt::Channel<'a, Blocking, Tx>) -> Self {
        Self {
            channel: Some(channel),
            framebuffer: [Srgb::new(0, 0, 0); LED_COUNT],
        }
    }

    /// Flush the framebuffer to the physical LEDs.
    pub async fn update(&mut self) {
        let Some(channel) = self.channel.take() else {
            error!("RMT channel lost during previous transmission");
            return;
        };

        let pulses = self
            .framebuffer
            .iter()
            .flat_map(|color| {
                let c: palette::rgb::Rgb<palette::encoding::Srgb, u8> = color.into_format::<u8>();
                // WS2812 expects GRB byte order
                [
                    Self::byte_to_pulses(c.green),
                    Self::byte_to_pulses(c.red),
                    Self::byte_to_pulses(c.blue),
                ]
                .into_iter()
                .flatten()
            })
            .chain(core::iter::once(PulseCode::end_marker()))
            .collect::<alloc::vec::Vec<_>>();

        let transaction = match channel.transmit(&pulses) {
            Ok(t) => t,
            Err(e) => {
                error!("RMT transmit failed: {}", e);
                return;
            }
        };

        self.channel = Some(match transaction.wait() {
            Ok(ch) => ch,
            Err((err, ch)) => {
                error!("RMT transaction failed: {}", err);
                ch
            }
        });

        // WS2812 reset time
        Timer::after(Duration::from_micros(50)).await;
    }

    /// Set a single LED by index.
    pub const fn set(&mut self, index: usize, color: Srgb<u8>) {
        self.framebuffer[index] = color;
    }

    /// Fill all LEDs with one colour.
    pub fn fill(&mut self, color: Srgb<u8>) {
        self.framebuffer.fill(color);
    }

    /// Turn all LEDs off.
    pub fn clear(&mut self) {
        self.fill(Srgb::new(0, 0, 0));
    }

    /// Fill LEDs from an iterator.
    pub fn fill_from_iter(&mut self, iter: impl IntoIterator<Item = Srgb<u8>>) {
        for (led, color) in self.framebuffer.iter_mut().zip(iter) {
            *led = color;
        }
    }

    /// Set the right LED bar (5 LEDs).
    ///
    /// Colors are ordered bottom-to-top: index 0 is the bottom LED,
    /// index 4 is the top LED. This matches the visual ordering of
    /// [`set_left_bar`], so passing the same array to both produces
    /// a symmetrical display.
    pub fn set_right_bar(&mut self, colors: &[Srgb<u8>; BAR_COUNT]) {
        // Right bar: hardware indices 0 (bottom) .. 4 (top) — already bottom-to-top.
        self.framebuffer[..BAR_COUNT].copy_from_slice(colors);
    }

    /// Set the left LED bar (5 LEDs).
    ///
    /// Colors are ordered bottom-to-top: index 0 is the bottom LED,
    /// index 4 is the top LED. Hardware indices 5–9 run top-to-bottom,
    /// so the slice is reversed internally.
    pub fn set_left_bar(&mut self, colors: &[Srgb<u8>; BAR_COUNT]) {
        // Left bar: hardware index 5 = top, 9 = bottom.
        // Reverse so that colors[0] = bottom, colors[4] = top.
        for i in 0..BAR_COUNT {
            self.framebuffer[BAR_COUNT + i] = colors[BAR_COUNT - 1 - i];
        }
    }

    /// Set both LED bars to the same colors.
    ///
    /// Convenience wrapper — equivalent to calling [`set_left_bar`] and
    /// [`set_right_bar`] with the same array.
    pub fn set_both_bars(&mut self, colors: &[Srgb<u8>; BAR_COUNT]) {
        self.set_right_bar(colors);
        self.set_left_bar(colors);
    }

    /// Number of LEDs on the strip.
    pub const fn len(&self) -> usize {
        LED_COUNT
    }

    // ── Internal helpers ────────────────────────────────────────────────

    /// WS2812 bit timing at 40 MHz RMT clock.
    const fn bit_to_pulse(bit: bool) -> PulseCode {
        if bit {
            // '1': 0.8 µs high (32 ticks), 0.45 µs low (18 ticks)
            PulseCode::new(Level::High, 32, Level::Low, 18)
        } else {
            // '0': 0.4 µs high (16 ticks), 0.85 µs low (34 ticks)
            PulseCode::new(Level::High, 16, Level::Low, 34)
        }
    }

    fn byte_to_pulses(byte: u8) -> [PulseCode; 8] {
        let mut pulses = [PulseCode::default(); 8];
        for (i, pulse) in pulses.iter_mut().enumerate() {
            *pulse = Self::bit_to_pulse((byte >> (7 - i)) & 1 != 0);
        }
        pulses
    }
}
