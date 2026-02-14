# Disobey 2026 badge in Rust

Easy mode hardware support library for the Disobey 2026 badge.

## Peripherals

| Peripheral | Type | Description |
|---|---|---|
| Display | ST7789 320×170 LCD | SPI + DMA, landscape orientation |
| Buttons | 9× GPIO inputs | D-pad, A/B, Start/Select, joystick click |
| LEDs | 10× WS2812 RGB | RMT-driven addressable strip |
| Backlight | GPIO output | Display backlight on/off |
| Vibration | GPIO output | Haptic feedback motor |

## Usage

Add as a dependency:

```toml
[dependencies]
disobey2026badge = { git = "https://github.com/tanelikaivola/disobey2026badge.git" }
```

```rust
let peripherals = disobey2026badge::init();
let resources = disobey2026badge::split_resources!(peripherals);

let display: disobey2026badge::Display = resources.display.into();
let buttons: disobey2026badge::Buttons = resources.buttons.into();
let leds: disobey2026badge::Leds = resources.leds.into();
let backlight: disobey2026badge::Backlight = resources.backlight.into();
let motor: disobey2026badge::Vibration = resources.vibra.into();
```

## Examples

```sh
cargo run --release --example <name>
```

### Games

| Example | Description |
|---|---|
| `breakout` | Breakout game with paddle, ball, and bricks. LEDs flash on brick hits. D-pad to move, A to launch |
| `skyroads` | Skyroads-style pseudo-3D game. Steer between lanes, jump over gaps and blocks, avoid tunnels. LEDs react to speed and state |
| `space_shooter` | Side-scrolling space shooter using ST7789 hardware scrolling for the background. D-pad to move, A to fire. Features weapon cycling, procedural nebula background, and LED feedback |

### Demos

| Example | Description |
|---|---|
| `demoscene` | Double-buffered dual-core demo cycling through plasma, starfield, copper bars, rotozoom, wireframe cube, tunnel, and warp effects with a sine scroller overlay |
| `shader` | Framebuffer-free shader demo streaming pixels directly to the display. Cycles through 12 effects: Julia set, plasma, tunnel, rotozoom, twisting tower, copper bars, fire, matrix rain, ripple, ray marching, voronoi, and warped checkerboard |
| `vectordemo` | Draws vector primitives directly to the display (no framebuffer). Randomly combines 11 effects: spinning fan, bouncing lines, Lissajous curves, rings, raster bars, starburst, starfield, wireframe cube, sine scope, bouncing balls, and spiral |

### Peripherals

| Example | Description |
|---|---|
| `backlight` | Toggles the display backlight on and off every second |
| `buttons` | Logs button presses via defmt — press any of the 9 buttons to see its name |
| `display` | Draws a color gradient and text on the ST7789 display, then blinks the backlight |
| `display_patterns` | Cycles through 25+ display test patterns: solid fills, color bars, gradients, checkerboards, grids, circles, text charts, noise, and more |
| `led_bars` | Demonstrates left/right LED bar functions: symmetric gradients, independent colors, and a scrolling dot |
| `leds` | Cycles a rainbow animation across all 10 WS2812 LEDs |
| `microphone` | Reads audio samples from the I2S microphone and logs peak amplitude (Except it's broken somehow, pull requests welcome)) |
| `nametag` | Displays a name scaled to fill the screen. Configurable via compile-time env vars: `NAME` (required), `BG`/`FG` (hex color or `BG="rainbow"`, BG="retrofuture" or BG="hearts"), `LEDS` (optional, `"heartbeat"` or `"rainbow"`) |
| `vertical_scroll` | Hardware vertical scrolling demo using display driver ST7789 with VSCRDEF/VSCRSADD commands to smoothly scroll colored stripes without redrawing |
| `vibration` | Pulses the vibration motor in a heartbeat pattern |

### Async

| Example | Description |
|---|---|
| `task_switch` | Two async tasks take turns drawing on the display using a Signal baton — a bouncing ball alternates with a scrolling text banner |

## Toolchain

Requires the `esp` Rust toolchain (`espup install`).

## It doesn't work!

Before panicing, try:

```
rustup update
espup update # or install
```
