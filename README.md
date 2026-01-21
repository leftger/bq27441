# BQ27441 Rust Driver

A `no_std` Rust driver for the Texas Instruments BQ27441-G1 battery fuel gauge IC.

## Features

- **I²C Communication**: Full support for blocking and async I²C operations
- **Comprehensive Battery Monitoring**:
  - State of Charge (SOC) - 0-100%
  - Voltage measurements (mV)
  - Current measurements (mA)
  - Temperature readings (°C or 0.1K)
  - Remaining capacity (mAh)
  - Full charge capacity (mAh)
  - State of Health (SOH) percentage
  - Average power (mW)
- **Power Management**: Support for NORMAL, SLEEP, HIBERNATE, and SHUTDOWN modes
- **Configuration**: Data Memory access for customization
- **Security**: SEALED/UNSEALED modes
- **YAML-Based Register Definitions**: Uses `device-driver` crate for type-safe register access
- **Async Support**: Optional async/await support via `embedded-hal-async`
- **Embassy Compatible**: Works seamlessly with the Embassy framework
- **defmt Support**: Optional defmt logging for debugging

## Hardware

The BQ27441-G1 is a System-Side Impedance Track™ Fuel Gauge for single-cell Li-Ion batteries.

- **I²C Address**: 0x55 (default)
- **Supply Voltage**: 1.8V - 3.6V
- **Sense Resistor**: Typically 10mΩ between SRN and SRP pins
- **Variants**:
  - BQ27441-G1A: For 4.2V maximum charge voltage batteries
  - BQ27441-G1B: For 4.3V or 4.35V maximum charge voltage batteries

## Usage

### Blocking I²C

```rust
use bq27441::Bq27441;
use embedded_hal::i2c::I2c;

// Create driver with default I2C address (0x55)
let mut gauge = Bq27441::new(i2c)?;

// Read battery status
let voltage = gauge.voltage()?;              // mV
let soc = gauge.state_of_charge()?;          // %
let capacity = gauge.remaining_capacity()?;  // mAh
let current = gauge.average_current()?;      // mA (signed)
let temp = gauge.temperature_celsius()?;     // °C

// Check charging status
if gauge.is_charging()? {
    println!("Battery is charging");
}

if gauge.is_full_charged()? {
    println!("Battery is fully charged");
}
```

### Async I²C (with Embassy)

```rust
use bq27441::Bq27441Async;
use embedded_hal_async::i2c::I2c;

// Create async driver
let mut gauge = Bq27441Async::new(i2c).await?;

// Read battery status asynchronously
let voltage = gauge.voltage().await?;
let soc = gauge.state_of_charge().await?;
let capacity = gauge.remaining_capacity().await?;
```

### Advanced Configuration

```rust
// Unseal device for configuration
gauge.unseal()?;

// Enter config update mode
gauge.enter_config_mode()?;

// Modify configuration via Data Memory
// (See datasheet for details)

// Exit config mode
gauge.exit_config_mode()?;

// Seal device to protect configuration
gauge.seal()?;
```

## Examples

See the `examples/` directory for complete working examples:

- `stm32wba65ri_embassy.rs` - Embassy async example for STM32WBA65RI

To run an example:

```bash
cargo build --example stm32wba65ri_embassy --features embassy --target thumbv8m.main-none-eabihf
```

## API Overview

### Core Methods

- `voltage()` - Read battery voltage in mV
- `state_of_charge()` - Read SOC percentage (0-100%)
- `remaining_capacity()` - Read remaining capacity in mAh
- `full_charge_capacity()` - Read full charge capacity in mAh
- `average_current()` - Read average current in mA (signed)
- `average_power()` - Read average power in mW (signed)
- `temperature_celsius()` - Read temperature in °C
- `state_of_health()` - Read SOH percentage

### Status Methods

- `is_battery_detected()` - Check if battery is connected
- `is_charging()` - Check if battery is charging
- `is_discharging()` - Check if battery is discharging
- `is_full_charged()` - Check if battery is fully charged
- `flags()` - Read all status flags

### Control Methods

- `control_read(cmd)` - Send control subcommand and read response
- `control_write(cmd)` - Send control subcommand
- `firmware_version()` - Read firmware version
- `chemistry_id()` - Read chemistry ID
- `seal()` - Enter SEALED mode
- `unseal()` - Exit SEALED mode
- `enter_config_mode()` - Enter CONFIG UPDATE mode
- `exit_config_mode()` - Exit CONFIG UPDATE mode
- `set_hibernate()` / `clear_hibernate()` - Hibernate mode control

## Features

Enable optional features in your `Cargo.toml`:

```toml
[dependencies]
bq27441 = { version = "0.1", features = ["async", "defmt-03"] }
```

Available features:
- `async` - Enable async/await support
- `defmt-03` - Enable defmt logging
- `embassy` - Enable both async and defmt (convenience feature)

## Register Access

For low-level register access, use the `.device()` method:

```rust
// Access generated device API directly
let flags = gauge.device().flags().read()?;
if flags.bat_det() {
    println!("Battery detected");
}
```

## Development

### Building

```bash
# Build for embedded target
cargo build --target thumbv8m.main-none-eabihf

# Build with async support
cargo build --target thumbv8m.main-none-eabihf --features async

# Build with Embassy + defmt
cargo build --target thumbv8m.main-none-eabihf --features embassy
```

### Register Definitions

Register definitions are maintained in `src/bq27441.yaml` using the `device-driver` crate's YAML format. The build script (`build.rs`) generates type-safe register access code at compile time.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## References

- [BQ27441-G1 Datasheet](https://www.ti.com/lit/ds/symlink/bq27441-g1.pdf)
- [BQ27441-G1 Technical Reference](https://www.ti.com/lit/ug/sluuac9a/sluuac9a.pdf)
- [device-driver crate](https://crates.io/crates/device-driver)
- [Embassy framework](https://embassy.dev/)
