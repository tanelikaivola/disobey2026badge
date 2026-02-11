//! 9-button input with async debouncing.
//!
//! The badge has a D-pad (up/down/left/right), A, B, Start, Select,
//! and a joystick click button.

use embassy_time::{
    Duration,
    Timer,
};
use esp_hal::gpio::{
    Input,
    InputConfig,
};

use crate::ButtonResources;

/// All nine badge buttons, ready for polling or async edge detection.
pub struct Buttons {
    pub up: Input<'static>,
    pub down: Input<'static>,
    pub left: Input<'static>,
    pub right: Input<'static>,
    pub stick: Input<'static>,
    pub a: Input<'static>,
    pub b: Input<'static>,
    pub start: Input<'static>,
    pub select: Input<'static>,
}

const DEBOUNCE_MS: u64 = 20;

impl From<ButtonResources<'static>> for Buttons {
    fn from(res: ButtonResources<'static>) -> Self {
        let pull_up = InputConfig::default().with_pull(esp_hal::gpio::Pull::Up);
        Self {
            up: Input::new(res.up, pull_up),
            down: Input::new(res.down, pull_up),
            left: Input::new(res.left, pull_up),
            right: Input::new(res.right, pull_up),
            stick: Input::new(res.stick, pull_up),
            a: Input::new(res.a, pull_up),
            b: Input::new(res.b, pull_up),
            start: Input::new(res.start, pull_up),
            select: Input::new(
                res.select,
                InputConfig::default().with_pull(esp_hal::gpio::Pull::Down),
            ),
        }
    }
}

impl Buttons {
    /// Wait for a full press-and-release cycle with debouncing.
    pub async fn debounce_press_and_release(button: &mut Input<'_>) {
        Self::debounce_press(button).await;
        Self::debounce_release(button).await;
    }

    /// Wait for a debounced button press (falling edge, active low).
    pub async fn debounce_press(button: &mut Input<'_>) {
        loop {
            button.wait_for_falling_edge().await;
            Timer::after(Duration::from_millis(DEBOUNCE_MS)).await;
            if button.is_low() {
                return;
            }
        }
    }

    /// Wait for a debounced button release (rising edge).
    pub async fn debounce_release(button: &mut Input<'_>) {
        loop {
            button.wait_for_rising_edge().await;
            Timer::after(Duration::from_millis(DEBOUNCE_MS)).await;
            if button.is_high() {
                return;
            }
        }
    }
}
