# disobey2026badge

Hardware support library for the Disobey 2026 badge (ESP32-S3).

## Peripherals

| Peripheral | Type | Description |
|---|---|---|
| Display | ST7789 320×170 LCD | SPI + DMA, landscape orientation |
| Buttons | 9× GPIO inputs | D-pad, A/B, Start/Select, joystick click |
| LEDs | 10× WS2812 RGB | RMT-driven addressable strip |
| Backlight | GPIO output | Display backlight on/off |
| Vibration | GPIO output | Haptic feedback motor |

## Usage

Add as a path dependency:

```toml
[dependencies]
disobey2026badge = { path = "../badge-minimal" }
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
cargo run --example buttons
cargo run --example leds
cargo run --example display
cargo run --example backlight
cargo run --example vibration
```

## Toolchain

Requires the `esp` Rust toolchain (`espup install`).
