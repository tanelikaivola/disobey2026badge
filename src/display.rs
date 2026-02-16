//! ST7789 display driver — 320×170 LCD over SPI with DMA.

use embedded_hal_bus::spi::ExclusiveDevice;
use esp_hal::{
    Async,
    dma::{
        DmaRxBuf,
        DmaTxBuf,
    },
    dma_buffers,
    gpio::{
        Level,
        Output,
        OutputConfig,
    },
    spi::master::Spi,
    time::Rate,
};

use crate::DisplayResources;

type SpiInterface<'a> = mipidsi::interface::SpiInterface<
    'a,
    ExclusiveDevice<esp_hal::spi::master::SpiDmaBus<'a, Async>, Output<'a>, esp_hal::delay::Delay>,
    Output<'a>,
>;

/// The badge's ST7789 display, ready to draw on with `embedded-graphics`.
pub type Display<'a> = mipidsi::Display<SpiInterface<'a>, mipidsi::models::ST7789, Output<'a>>;

/// StaticCell helper (local to this module to avoid macro import issues).
macro_rules! mk_static {
    ($t:ty, $val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write($val);
        x
    }};
}

impl<'a> From<DisplayResources<'a>> for Display<'a> {
    fn from(res: DisplayResources<'a>) -> Self {
        let (rx_buffer, rx_descriptors, tx_buffer, tx_descriptors) = dma_buffers!(32000);
        let dma_rx_buf = DmaRxBuf::new(rx_descriptors, rx_buffer).unwrap();
        let dma_tx_buf = DmaTxBuf::new(tx_descriptors, tx_buffer).unwrap();

        let mut delay = esp_hal::delay::Delay::new();

        let dc = Output::new(res.dc, Level::Low, OutputConfig::default());
        let mut rst = Output::new(res.rst, Level::Low, OutputConfig::default());
        rst.set_high();

        let spi = Spi::new(
            res.spi,
            esp_hal::spi::master::Config::default().with_frequency(Rate::from_mhz(80)),
        )
        .unwrap()
        .with_sck(res.sck)
        .with_mosi(res.mosi)
        .with_miso(res.miso)
        .with_dma(res.dma)
        .with_buffers(dma_rx_buf, dma_tx_buf)
        .into_async();

        let cs = Output::new(res.cs, Level::High, OutputConfig::default());
        let spi_device = ExclusiveDevice::new(spi, cs, delay).unwrap();

        let buffer = mk_static!([u8; 32000], [0_u8; 32000]);
        let di = mipidsi::interface::SpiInterface::new(spi_device, dc, buffer);

        mipidsi::Builder::new(mipidsi::models::ST7789, di)
            .reset_pin(rst)
            .display_size(170, 320)
            .invert_colors(mipidsi::options::ColorInversion::Inverted)
            .orientation(
                mipidsi::options::Orientation::new().rotate(mipidsi::options::Rotation::Deg90),
            )
            .display_offset(35, 0)
            .init(&mut delay)
            .unwrap()
    }
}
