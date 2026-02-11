//! # disobey2026badge
//!
//! Hardware support library for the Disobey 2026 badge.
//!
//! Provides clean abstractions for all onboard peripherals:
//! - **Display**: 320×170 ST7789 LCD over SPI with DMA
//! - **Buttons**: 9-button input (D-pad, A/B, Start/Select, joystick click) with debouncing
//! - **LEDs**: 10× WS2812 addressable RGB LEDs via RMT
//! - **Backlight**: Display backlight control
//! - **Vibration motor**: Haptic feedback
//! - **Microphone**: I2S MEMS microphone input
//!
//! ## Quick start
//!
//! ```rust,ignore
//! let peripherals = disobey2026badge::init();
//! let resources = disobey2026badge::split_resources!(peripherals);
//!
//! let display: disobey2026badge::Display = resources.display.into();
//! let buttons: disobey2026badge::Buttons = resources.buttons.into();
//! let leds: disobey2026badge::Leds = resources.leds.into();
//! ```

#![no_std]

mod backlight;
mod buttons;
mod display;
mod leds;
pub mod microphone;
mod vibration;

pub use backlight::Backlight;
pub use buttons::Buttons;
pub use display::Display;
use esp_hal::{
    Async,
    Blocking,
    assign_resources,
    clock::{
        Clock,
        CpuClock,
    },
    gpio::{
        Level,
        Output,
        OutputConfig,
    },
    rmt::{
        Rmt,
        Tx,
        TxChannelConfig,
        TxChannelCreator as _,
    },
    rom,
    time::Rate,
};
pub use leds::{
    BAR_COUNT,
    Leds,
};
pub use microphone::Microphone;
pub use vibration::Vibration;

/// StaticCell helper — allocates a value into a `static` exactly once.
#[macro_export]
macro_rules! mk_static {
    ($t:ty, $val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write($val);
        x
    }};
}

// ── Pin / peripheral assignments ────────────────────────────────────────────

assign_resources! {
    pub Resources<'d> {
        display: DisplayResources<'d> {
            dc: GPIO15,
            rst: GPIO7,
            sck: GPIO4,
            cs: GPIO6,
            miso: GPIO16,
            mosi: GPIO5,
            spi: SPI2,
            dma: DMA_CH0,
        },
        backlight: BacklightResources<'d> {
            led: GPIO19,
        },
        buttons: ButtonResources<'d> {
            up: GPIO11,
            down: GPIO1,
            left: GPIO21,
            right: GPIO2,
            stick: GPIO14,
            a: GPIO13,
            b: GPIO38,
            start: GPIO12,
            select: GPIO45,
        },
        leds: LedResources<'d> {
            power: GPIO17,
            io: GPIO18,
            rmt: RMT,
        },
        vibra: VibrationResources<'d> {
            motor: GPIO20,
        },
        mic: MicResources<'d> {
            ws: GPIO8,
            sd: GPIO3,
            dio: GPIO46,
            i2s: I2S0,
            dma: DMA_CH1,
        },
        boot: BootResources<'d> {
            pin: GPIO0,
        }
    }
}

// ── Board initialisation ────────────────────────────────────────────────────

/// Minimal CPU clock switcher for ESP32-S3.
///
/// Steps through an intermediate frequency before reaching the target,
/// which is required by the hardware.
fn set_cpu_clock(cpu_clock_speed: CpuClock) {
    let _ = esp_hal::peripherals::SYSTEM::regs()
        .sysclk_conf()
        .modify(|_, w| unsafe { w.soc_clk_sel().bits(1) });
    let _ = esp_hal::peripherals::SYSTEM::regs()
        .cpu_per_conf()
        .modify(|_, w| unsafe {
            let _ = w.pll_freq_sel().set_bit();
            w.cpuperiod_sel().bits(match cpu_clock_speed {
                CpuClock::_80MHz => 0,
                CpuClock::_160MHz => 1,
                CpuClock::_240MHz => 2,
                _ => panic!("Unsupported CPU clock speed"),
            })
        });

    rom::ets_update_cpu_frequency_rom(cpu_clock_speed.frequency().as_mhz());
}

/// Initialise the badge hardware and return the raw peripheral set.
///
/// Call this once at the top of your `main`. Then use [`split_resources!`] to
/// break the peripherals into typed resource groups.
#[must_use]
pub fn init() -> esp_hal::peripherals::Peripherals {
    set_cpu_clock(CpuClock::_160MHz);
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    esp_hal::init(config)
}

// ── Resource → peripheral conversions ───────────────────────────────────────

impl From<esp_hal::peripherals::Peripherals> for Resources<'_> {
    fn from(peripherals: esp_hal::peripherals::Peripherals) -> Self {
        split_resources!(peripherals)
    }
}

impl<'a> From<LedResources<'a>> for esp_hal::rmt::Channel<'a, Blocking, Tx> {
    fn from(res: LedResources<'a>) -> Self {
        let _ws_power = Output::new(res.power, Level::High, OutputConfig::default());
        let rmt = Rmt::new(res.rmt, Rate::from_mhz(40)).unwrap();
        let tx_config = TxChannelConfig::default().with_clk_divider(1);
        rmt.channel0.configure_tx(res.io, tx_config).unwrap()
    }
}

impl<'a> From<LedResources<'a>> for esp_hal::rmt::Channel<'a, Async, Tx> {
    fn from(res: LedResources<'a>) -> Self {
        let _ws_power = Output::new(res.power, Level::High, OutputConfig::default());
        let rmt = Rmt::new(res.rmt, Rate::from_mhz(40)).unwrap().into_async();
        let tx_config = TxChannelConfig::default().with_clk_divider(1);
        rmt.channel0.configure_tx(res.io, tx_config).unwrap()
    }
}

impl<'a> From<LedResources<'a>> for Leds<'a> {
    fn from(res: LedResources<'a>) -> Self {
        Leds::new(res.into())
    }
}
