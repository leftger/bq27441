# BQ27441 Rust Driver

[![crates.io](https://img.shields.io/crates/v/bq27441.svg)](https://crates.io/crates/bq27441)
[![docs.rs](https://img.shields.io/docsrs/bq27441)](https://docs.rs/bq27441)
[![CI](https://github.com/leftger/bq27441/actions/workflows/ci.yml/badge.svg)](https://github.com/leftger/bq27441/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)

A `no_std` Rust driver for the Texas Instruments BQ27441-G1 battery fuel gauge IC.

## Features

- **I²C Communication**: Full support for blocking and async I²C operations
- **Comprehensive Battery Monitoring**:
  - State of Charge (SOC) - 0-100% (filtered/unfiltered)
  - Voltage measurements (mV)
  - Current measurements (mA)
  - Temperature readings (°C or 0.1K)
  - Remaining capacity and full charge capacity (mAh, filtered/unfiltered)
  - State of Health (SOH) percentage
  - Average power (mW)
- **Power Management**: Support for NORMAL, SLEEP, HIBERNATE, and SHUTDOWN modes
- **Battery Configuration Workflow**: Typed `BatteryConfig` (design capacity, OpConfig, current
  thresholds, safety/discharge/charge-termination alarms, Ra table) applied in one call via
  `configure_battery()`, which drives the full unseal → CONFIG UPDATE → apply → exit → reseal
  sequence for you
- **Golden-File Snapshots**: Read/write the calibrated Data Memory block set (`GoldenSnapshot`)
  used to back up or clone a tuned device
- **Learning-Cycle Monitoring**: `learning_progress()` and related helpers to track Impedance
  Track™ Qmax/Ra learning status
- **Raw Data Memory Access**: Block- and subclass-level read/write escape hatches for anything
  not covered by the typed config API
- **Security**: SEALED/UNSEALED modes
- **YAML-Based Register Definitions**: Uses the `device-driver` crate for type-safe register access
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
let current = gauge.average_current()?;     // mA (signed)
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

### Configuring a New Battery

`configure_battery()` runs the whole CONFIG UPDATE workflow (unseal, enter config mode, write
Data Memory, commit checksums, exit, reseal) in one call:

```rust
use bq27441::{BatteryConfig, ChemId, ConfigureOptions, BusyWait};

let config = BatteryConfig::for_chemistry(ChemId::G1A);
gauge.configure_battery(&config, ConfigureOptions::default(), &mut BusyWait)?;
```

Start from `BatteryConfig::G1A_DEFAULT` / `BatteryConfig::G1B_DEFAULT` (or `for_chemistry`) and
override fields such as `design_capacity_mah`, `current_thresholds`, `safety`, `discharge`,
`charge_termination`, or `ra_table` as needed. Use `configure_battery_default()` if you don't
have your own `DelayMs` implementation. For manual control over the sequence, combine `unseal()`,
`enter_config_mode()`, `apply_battery_config()`, and `exit_config()` yourself.

### Golden-File Snapshots

```rust
// Back up a calibrated device
let snapshot = gauge.read_golden_snapshot()?;

// ...later, restore it onto another unit (device must be in CONFIG UPDATE mode)
gauge.write_golden_snapshot(snapshot, &mut BusyWait)?;
```

### Learning-Cycle Monitoring

```rust
let progress = gauge.learning_progress()?;
if let Some(phase) = progress.phase() {
    println!("Learning phase: {phase:?}");
}
```

### Manual Seal/Config-Mode Control

```rust
// Unseal device for configuration
gauge.unseal()?;

// Enter config update mode
gauge.enter_config_mode()?;

// Modify configuration via Data Memory
// (see `configure_battery`/`BatteryConfig` above, or use the raw
// `read_data_memory_block`/`write_data_memory_block` escape hatches)

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
- `state_of_charge()` / `state_of_charge_unfiltered()` - Read SOC percentage (0-100%)
- `remaining_capacity()` / `remaining_capacity_unfiltered()` / `remaining_capacity_filtered()` - Remaining capacity in mAh
- `full_charge_capacity()` / `full_charge_capacity_unfiltered()` / `full_charge_capacity_filtered()` - Full charge capacity in mAh
- `nominal_available_capacity()` / `full_available_capacity()` - Unfiltered capacity readings in mAh
- `average_current()` - Read average current in mA (signed)
- `average_power()` - Read average power in mW (signed)
- `standby_current()` / `max_load_current()` - Additional current readings in mA
- `temperature_celsius()` / `internal_temperature_celsius()` - Temperature in °C
- `state_of_health()` / `state_of_health_status()` - SOH percentage / decoded status

### Status Methods

- `is_battery_detected()` - Check if battery is connected
- `is_charging()` / `is_discharging()` - Check charge direction
- `is_full_charged()` - Check if battery is fully charged
- `is_over_temp()` / `is_under_temp()` - Check temperature alarms
- `needs_config_reload()` - Check whether Data Memory needs to be re-applied
- `flags()` - Read all status flags

### Control & Lifecycle Methods

- `control_read(cmd)` / `control_write(cmd)` - Send raw control subcommands
- `control_status()` / `is_sealed()` - Read decoded `CONTROL_STATUS`
- `firmware_version()` - Read firmware version
- `chemistry_id()` - Read chemistry ID (`ChemId::G1A`/`G1B`)
- `seal()` / `unseal()` - Enter/exit SEALED mode
- `enter_config_mode()` / `exit_config_mode()` / `exit_config(ConfigExit)` - CONFIG UPDATE mode control
- `bat_insert()` / `bat_remove()` - Battery presence signaling
- `set_hibernate()` / `clear_hibernate()` - Hibernate mode control
- `shutdown_enable()` / `shutdown()` - Shutdown mode control
- `reset()` / `soft_reset()` / `pulse_gpout()` - Device reset and GPOUT control

### Configuration Methods

- `read_battery_config()` - Read the current `BatteryConfig` from Data Memory (unsealed)
- `apply_battery_config(config)` - Write a `BatteryConfig` while already in CONFIG UPDATE mode
- `configure_battery(config, options, &mut delay)` / `configure_battery_default(config, options)` - Full unseal/apply/reseal workflow
- `read_golden_snapshot()` / `write_golden_snapshot(snapshot, &mut delay)` - Back up/restore calibrated Data Memory
- `read_data_memory_block()` / `write_data_memory_block()` / `read_data_memory_subclass()` / `write_data_memory_subclass()` - Raw block/subclass access
- `learning_progress()` / `update_status()` / `qmax_cell_0()` / `delta_voltage_mv()` - Learning-cycle monitoring

## Features

Enable optional features in your `Cargo.toml`:

```toml
[dependencies]
bq27441 = { version = "0.2", features = ["async", "defmt-03"] }
```

Available features:
- `async` - Enable async/await support (`Bq27441Async` and async config/golden/learning helpers)
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

The contents of this repository are dual-licensed under the _MIT OR Apache 2.0_
License. That means you can choose either the MIT license or the Apache 2.0
license when you re-use this code. See [`LICENSE-MIT`](./LICENSE-MIT) or
[`LICENSE-APACHE`](./LICENSE-APACHE) for more information on each specific
license.

## References

- [BQ27441-G1 Datasheet](https://www.ti.com/lit/ds/symlink/bq27441-g1.pdf)
- [BQ27441-G1 Technical Reference](https://www.ti.com/lit/ug/sluuac9a/sluuac9a.pdf)
- [device-driver crate](https://crates.io/crates/device-driver)
- [Embassy framework](https://embassy.dev/)
