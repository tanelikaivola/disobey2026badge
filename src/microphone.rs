//! I2S MEMS microphone driver.
//!
//! The badge has an I2S microphone connected via:
//! - WS (word select / LRCLK) on GPIO8
//! - SD (serial data / DIN) on GPIO3
//! - DIO (bit clock / BCLK) on GPIO46
//!
//! Uses DMA for efficient sample capture.

use esp_hal::{
    Blocking,
    dma::DmaDescriptor,
    i2s::master::{
        Channels,
        Config,
        DataFormat,
        I2s,
        I2sRx,
    },
    time::Rate,
};

use crate::MicResources;

/// Default sample rate for the microphone (16 kHz).
pub const DEFAULT_SAMPLE_RATE: u32 = 16_000;

/// I2S microphone, ready for DMA reads.
pub struct Microphone<'a> {
    pub rx: I2sRx<'a, Blocking>,
}

impl<'a> Microphone<'a> {
    /// Create a new microphone from raw resources and a static descriptor slice.
    ///
    /// `sample_rate` is in Hz (e.g. 16000 for 16 kHz).
    /// `descriptors` must be a `&'static mut` slice — use [`mk_static!`](crate::mk_static)
    /// or a static array.
    pub fn new(
        res: MicResources<'a>,
        sample_rate: u32,
        descriptors: &'static mut [DmaDescriptor],
    ) -> Self {
        let i2s = I2s::new(
            res.i2s,
            res.dma,
            Config::new_tdm_philips()
                .with_sample_rate(Rate::from_hz(sample_rate))
                .with_data_format(DataFormat::Data16Channel16)
                .with_channels(Channels::MONO),
        )
        .unwrap();

        let rx = i2s
            .i2s_rx
            .with_bclk(res.dio)
            .with_ws(res.ws)
            .with_din(res.sd)
            .build(descriptors);

        Self { rx }
    }
}
