//! Display backlight control.

use esp_hal::gpio::{
    Level,
    Output,
    OutputConfig,
};

use crate::BacklightResources;

/// Controls the display backlight LED.
pub struct Backlight {
    pin: Output<'static>,
}

impl From<BacklightResources<'static>> for Backlight {
    fn from(res: BacklightResources<'static>) -> Self {
        // Default to backlight ON
        Self {
            pin: Output::new(res.led, Level::High, OutputConfig::default()),
        }
    }
}

impl Backlight {
    pub fn on(&mut self) {
        self.pin.set_high();
    }

    pub fn off(&mut self) {
        self.pin.set_low();
    }

    pub fn toggle(&mut self) {
        self.pin.toggle();
    }

    pub fn is_on(&self) -> bool {
        self.pin.is_set_high()
    }
}
