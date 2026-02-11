//! Vibration motor control for haptic feedback.

use embassy_time::{
    Duration,
    Timer,
};
use esp_hal::gpio::{
    Level,
    Output,
    OutputConfig,
};

use crate::VibrationResources;

/// Controls the onboard vibration motor.
pub struct Vibration {
    pin: Output<'static>,
}

impl From<VibrationResources<'static>> for Vibration {
    fn from(res: VibrationResources<'static>) -> Self {
        Self {
            pin: Output::new(res.motor, Level::Low, OutputConfig::default()),
        }
    }
}

impl Vibration {
    pub fn on(&mut self) {
        self.pin.set_high();
    }

    pub fn off(&mut self) {
        self.pin.set_low();
    }

    /// Buzz for the given duration, then stop.
    pub async fn pulse(&mut self, duration: Duration) {
        self.on();
        Timer::after(duration).await;
        self.off();
    }
}
