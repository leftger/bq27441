//! High-level BQ27441 battery fuel gauge driver with synchronous and asynchronous support.
//!
//! This driver provides access to the BQ27441-G1 battery fuel gauge IC from Texas Instruments.
//! It supports I²C communication with both blocking and async modes.
//!
//! # Features
//!
//! - State of Charge (SOC) monitoring
//! - Voltage, current, and temperature measurements
//! - Remaining capacity and full charge capacity reporting
//! - Power and current averaging
//! - State of Health (SOH) reporting
//! - Configuration via Data Memory
//! - SEALED/UNSEALED access modes
//! - Multiple power modes (NORMAL, SLEEP, HIBERNATE, SHUTDOWN)
//!
//! # Examples
//!
//! ## Blocking I2C
//!
//! ```no_run
//! use bq27441::Bq27441;
//! # use embedded_hal::i2c::I2c;
//! # fn example<I2C: I2c>(i2c: I2C) -> Result<(), bq27441::Error<I2C::Error>>
//! # where I2C::Error: core::fmt::Debug {
//!
//! // Create driver with default I2C address (0x55)
//! let mut gauge = Bq27441::new(i2c)?;
//!
//! // Read battery voltage
//! let voltage_mv = gauge.voltage()?;
//!
//! // Read state of charge (0-100%)
//! let soc = gauge.state_of_charge()?;
//!
//! // Read remaining capacity
//! let capacity_mah = gauge.remaining_capacity()?;
//!
//! # Ok(())
//! # }
//! ```
//!
//! ## Async I2C
//!
//! ```no_run
//! use bq27441::Bq27441Async;
//! # use embedded_hal_async::i2c::I2c;
//! # async fn example<I2C: I2c>(i2c: I2C) -> Result<(), bq27441::Error<I2C::Error>>
//! # where I2C::Error: core::fmt::Debug {
//!
//! // Create async driver
//! let mut gauge = Bq27441Async::new(i2c).await?;
//!
//! // Read battery status asynchronously
//! let voltage = gauge.voltage().await?;
//! let soc = gauge.state_of_charge().await?;
//!
//! # Ok(())
//! # }
//! ```

#![no_std]
#![deny(missing_docs)]
#![deny(warnings)]
#![allow(clippy::missing_errors_doc)]

use core::fmt::Debug;

#[cfg(feature = "async")]
use device_driver::AsyncRegisterInterface;
use device_driver::RegisterInterface;
use embedded_hal as hal;
#[cfg(feature = "async")]
use embedded_hal_async as hal_async;

#[allow(unsafe_code)]
#[allow(missing_docs)]
#[allow(clippy::doc_markdown, clippy::missing_errors_doc, clippy::identity_op)]
mod generated {
    device_driver::create_device!(
        device_name: Bq27441Device,
        manifest: "src/bq27441.yaml"
    );
}

pub use generated::{Bq27441Device, field_sets};

/// Default I²C address for BQ27441.
pub const DEFAULT_I2C_ADDRESS: u8 = 0x55;

/// Unseal key value (sent twice to unseal device).
pub const UNSEAL_KEY: u16 = 0x8000;

/// Expected device type ID.
pub const DEVICE_TYPE_ID: u16 = 0x0421;

/// Control subcommand codes.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlCmd {
    /// Read control status.
    ControlStatus = 0x0000,
    /// Read device type (should return 0x0421).
    DeviceType = 0x0001,
    /// Read firmware version.
    FwVersion = 0x0002,
    /// Read data memory code.
    DmCode = 0x0004,
    /// Read previous MAC write command.
    PrevMacWrite = 0x0007,
    /// Read chemistry ID.
    ChemId = 0x0008,
    /// Signal battery insertion.
    BatInsert = 0x000C,
    /// Signal battery removal.
    BatRemove = 0x000D,
    /// Set hibernate mode.
    SetHibernate = 0x0011,
    /// Clear hibernate mode.
    ClearHibernate = 0x0012,
    /// Enter config update mode.
    SetCfgUpdate = 0x0013,
    /// Enable shutdown mode.
    ShutdownEnable = 0x001B,
    /// Enter shutdown mode.
    Shutdown = 0x001C,
    /// Enter sealed mode.
    Sealed = 0x0020,
    /// Pulse GPOUT pin.
    PulseGpout = 0x0023,
    /// Full device reset.
    Reset = 0x0041,
    /// Soft reset (exit config mode with OCV).
    SoftReset = 0x0042,
    /// Exit config mode without OCV.
    ExitCfgUpdate = 0x0043,
    /// Exit config mode with resimulation.
    ExitResim = 0x0044,
}

/// Error type for BQ27441 operations.
#[derive(Debug)]
pub enum Error<E> {
    /// I²C bus error.
    I2c(E),
    /// Device ID mismatch.
    InvalidDevice,
    /// Invalid parameter.
    InvalidParam,
}

impl<E> From<E> for Error<E> {
    fn from(err: E) -> Self {
        Error::I2c(err)
    }
}

/// Battery chemistry variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChemId {
    /// G1A variant (4.2V max charge, `CHEM_ID` = 0x0128).
    G1A = 0x0128,
    /// G1B variant (4.3V/4.35V max charge, `CHEM_ID` = 0x0312).
    G1B = 0x0312,
}

/// Blocking I²C interface wrapper.
pub struct DeviceInterface<I2C> {
    /// Underlying I²C bus.
    pub i2c: I2C,
    /// Device I²C address.
    pub address: u8,
}

/// Asynchronous I²C interface wrapper.
#[cfg(feature = "async")]
pub struct DeviceInterfaceAsync<I2C> {
    /// Underlying async I²C bus.
    pub i2c: I2C,
    /// Device I²C address.
    pub address: u8,
}

impl<I2C> RegisterInterface for DeviceInterface<I2C>
where
    I2C: hal::i2c::I2c,
{
    type Error = I2C::Error;
    type AddressType = u8;

    fn write_register(
        &mut self,
        address: Self::AddressType,
        _size_bits: u32,
        data: &[u8],
    ) -> Result<(), Self::Error> {
        let mut buf = [0u8; 1 + 8];
        buf[0] = address;
        let end = 1 + data.len();
        buf[1..end].copy_from_slice(data);
        self.i2c.write(self.address, &buf[..end])
    }

    fn read_register(
        &mut self,
        address: Self::AddressType,
        _size_bits: u32,
        data: &mut [u8],
    ) -> Result<(), Self::Error> {
        self.i2c.write_read(self.address, &[address], data)
    }
}

#[cfg(feature = "async")]
impl<I2C> AsyncRegisterInterface for DeviceInterfaceAsync<I2C>
where
    I2C: hal_async::i2c::I2c,
{
    type Error = I2C::Error;
    type AddressType = u8;

    async fn write_register(
        &mut self,
        address: Self::AddressType,
        _size_bits: u32,
        data: &[u8],
    ) -> Result<(), Self::Error> {
        let mut buf = [0u8; 1 + 8];
        buf[0] = address;
        let end = 1 + data.len();
        buf[1..end].copy_from_slice(data);
        self.i2c.write(self.address, &buf[..end]).await
    }

    async fn read_register(
        &mut self,
        address: Self::AddressType,
        _size_bits: u32,
        data: &mut [u8],
    ) -> Result<(), Self::Error> {
        self.i2c.write_read(self.address, &[address], data).await
    }
}

/// Blocking BQ27441 driver.
pub struct Bq27441<I2C> {
    device: Bq27441Device<DeviceInterface<I2C>>,
}

impl<I2C> Bq27441<I2C>
where
    I2C: hal::i2c::I2c,
    I2C::Error: Debug,
{
    /// Create a new driver with default I²C address (0x55).
    pub fn new(i2c: I2C) -> Result<Self, Error<I2C::Error>> {
        Self::new_with_address(i2c, DEFAULT_I2C_ADDRESS)
    }

    /// Create a new driver with a custom I²C address.
    pub fn new_with_address(i2c: I2C, address: u8) -> Result<Self, Error<I2C::Error>> {
        let interface = DeviceInterface { i2c, address };
        let device = Bq27441Device::new(interface);
        let mut this = Self { device };
        this.verify_device()?;
        Ok(this)
    }

    /// Verify device ID matches expected value.
    fn verify_device(&mut self) -> Result<(), Error<I2C::Error>> {
        let device_type = self.control_read(ControlCmd::DeviceType)?;
        if device_type != DEVICE_TYPE_ID {
            return Err(Error::InvalidDevice);
        }
        Ok(())
    }

    /// Send a control subcommand and read the 2-byte response.
    pub fn control_read(&mut self, cmd: ControlCmd) -> Result<u16, Error<I2C::Error>> {
        // Write subcommand to control register
        let cmd_bytes = (cmd as u16).to_le_bytes();
        self.device
            .control()
            .write(|w| *w = field_sets::Control::from(cmd_bytes))
            .map_err(Error::I2c)?;

        // Small delay to allow device to process command
        // (In real implementation, might want to use a timer)

        // Read response from control register
        let result = self.device.control().read().map_err(Error::I2c)?;
        let bytes: [u8; 2] = result.into();
        Ok(u16::from_le_bytes(bytes))
    }

    /// Send a control subcommand (write-only, no response).
    pub fn control_write(&mut self, cmd: ControlCmd) -> Result<(), Error<I2C::Error>> {
        let cmd_bytes = (cmd as u16).to_le_bytes();
        self.device
            .control()
            .write(|w| *w = field_sets::Control::from(cmd_bytes))
            .map_err(Error::I2c)
    }

    /// Read battery voltage in millivolts.
    pub fn voltage(&mut self) -> Result<u16, Error<I2C::Error>> {
        let val = self.device.voltage().read().map_err(Error::I2c)?;
        let bytes: [u8; 2] = val.into();
        Ok(u16::from_le_bytes(bytes))
    }

    /// Read temperature in 0.1 Kelvin units.
    pub fn temperature_raw(&mut self) -> Result<u16, Error<I2C::Error>> {
        let val = self.device.temperature().read().map_err(Error::I2c)?;
        let bytes: [u8; 2] = val.into();
        Ok(u16::from_le_bytes(bytes))
    }

    /// Read temperature in degrees Celsius.
    pub fn temperature_celsius(&mut self) -> Result<f32, Error<I2C::Error>> {
        let raw = self.temperature_raw()?;
        Ok((f32::from(raw) * 0.1) - 273.15)
    }

    /// Read state of charge (0-100%).
    pub fn state_of_charge(&mut self) -> Result<u16, Error<I2C::Error>> {
        let val = self.device.state_of_charge().read().map_err(Error::I2c)?;
        let bytes: [u8; 2] = val.into();
        Ok(u16::from_le_bytes(bytes))
    }

    /// Read remaining capacity in mAh.
    pub fn remaining_capacity(&mut self) -> Result<u16, Error<I2C::Error>> {
        let val = self
            .device
            .remaining_capacity()
            .read()
            .map_err(Error::I2c)?;
        let bytes: [u8; 2] = val.into();
        Ok(u16::from_le_bytes(bytes))
    }

    /// Read full charge capacity in mAh.
    pub fn full_charge_capacity(&mut self) -> Result<u16, Error<I2C::Error>> {
        let val = self
            .device
            .full_charge_capacity()
            .read()
            .map_err(Error::I2c)?;
        let bytes: [u8; 2] = val.into();
        Ok(u16::from_le_bytes(bytes))
    }

    /// Read average current in mA (signed).
    pub fn average_current(&mut self) -> Result<i16, Error<I2C::Error>> {
        let val = self.device.average_current().read().map_err(Error::I2c)?;
        let bytes: [u8; 2] = val.into();
        Ok(i16::from_le_bytes(bytes))
    }

    /// Read average power in mW (signed).
    pub fn average_power(&mut self) -> Result<i16, Error<I2C::Error>> {
        let val = self.device.average_power().read().map_err(Error::I2c)?;
        let bytes: [u8; 2] = val.into();
        Ok(i16::from_le_bytes(bytes))
    }

    /// Read state of health percentage.
    pub fn state_of_health(&mut self) -> Result<u8, Error<I2C::Error>> {
        let val = self.device.state_of_health().read().map_err(Error::I2c)?;
        let bytes: [u8; 2] = val.into();
        Ok(bytes[0]) // Lower byte is percentage
    }

    /// Read status flags.
    pub fn flags(&mut self) -> Result<field_sets::Flags, Error<I2C::Error>> {
        self.device.flags().read().map_err(Error::I2c)
    }

    /// Check if battery is detected.
    pub fn is_battery_detected(&mut self) -> Result<bool, Error<I2C::Error>> {
        let flags = self.flags()?;
        Ok(flags.bat_det())
    }

    /// Check if battery is charging.
    pub fn is_charging(&mut self) -> Result<bool, Error<I2C::Error>> {
        let flags = self.flags()?;
        Ok(flags.chg())
    }

    /// Check if battery is discharging.
    pub fn is_discharging(&mut self) -> Result<bool, Error<I2C::Error>> {
        let flags = self.flags()?;
        Ok(flags.dsg())
    }

    /// Check if battery is fully charged.
    pub fn is_full_charged(&mut self) -> Result<bool, Error<I2C::Error>> {
        let flags = self.flags()?;
        Ok(flags.fc())
    }

    /// Read firmware version.
    pub fn firmware_version(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.control_read(ControlCmd::FwVersion)
    }

    /// Read chemistry ID.
    pub fn chemistry_id(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.control_read(ControlCmd::ChemId)
    }

    /// Enter SEALED mode (protects configuration).
    pub fn seal(&mut self) -> Result<(), Error<I2C::Error>> {
        self.control_write(ControlCmd::Sealed)
    }

    /// Unseal the device (allows configuration changes).
    /// Requires sending the unseal key (0x8000) twice.
    pub fn unseal(&mut self) -> Result<(), Error<I2C::Error>> {
        let key_bytes = UNSEAL_KEY.to_le_bytes();
        // Send key first time
        self.device
            .control()
            .write(|w| *w = field_sets::Control::from(key_bytes))
            .map_err(Error::I2c)?;
        // Send key second time
        self.device
            .control()
            .write(|w| *w = field_sets::Control::from(key_bytes))
            .map_err(Error::I2c)
    }

    /// Enter config update mode (device must be unsealed).
    pub fn enter_config_mode(&mut self) -> Result<(), Error<I2C::Error>> {
        self.control_write(ControlCmd::SetCfgUpdate)
    }

    /// Exit config update mode with soft reset.
    pub fn exit_config_mode(&mut self) -> Result<(), Error<I2C::Error>> {
        self.control_write(ControlCmd::SoftReset)
    }

    /// Perform a soft reset.
    pub fn soft_reset(&mut self) -> Result<(), Error<I2C::Error>> {
        self.control_write(ControlCmd::SoftReset)
    }

    /// Set hibernate mode.
    pub fn set_hibernate(&mut self) -> Result<(), Error<I2C::Error>> {
        self.control_write(ControlCmd::SetHibernate)
    }

    /// Clear hibernate mode.
    pub fn clear_hibernate(&mut self) -> Result<(), Error<I2C::Error>> {
        self.control_write(ControlCmd::ClearHibernate)
    }

    /// Access the generated register API directly.
    pub fn device(&mut self) -> &mut Bq27441Device<DeviceInterface<I2C>> {
        &mut self.device
    }

    /// Consume the driver and return the underlying I²C bus.
    pub fn destroy(self) -> I2C {
        self.device.interface.i2c
    }
}

/// Asynchronous BQ27441 driver.
#[cfg(feature = "async")]
pub struct Bq27441Async<I2C> {
    device: Bq27441Device<DeviceInterfaceAsync<I2C>>,
}

#[cfg(feature = "async")]
impl<I2C> Bq27441Async<I2C>
where
    I2C: hal_async::i2c::I2c,
    I2C::Error: Debug,
{
    /// Create a new async driver with default I²C address (0x55).
    pub async fn new(i2c: I2C) -> Result<Self, Error<I2C::Error>> {
        Self::new_with_address(i2c, DEFAULT_I2C_ADDRESS).await
    }

    /// Create a new async driver with a custom I²C address.
    pub async fn new_with_address(i2c: I2C, address: u8) -> Result<Self, Error<I2C::Error>> {
        let interface = DeviceInterfaceAsync { i2c, address };
        let device = Bq27441Device::new(interface);
        let mut this = Self { device };
        this.verify_device().await?;
        Ok(this)
    }

    /// Verify device ID matches expected value.
    async fn verify_device(&mut self) -> Result<(), Error<I2C::Error>> {
        let device_type = self.control_read(ControlCmd::DeviceType).await?;
        if device_type != DEVICE_TYPE_ID {
            return Err(Error::InvalidDevice);
        }
        Ok(())
    }

    /// Send a control subcommand and read the 2-byte response.
    pub async fn control_read(&mut self, cmd: ControlCmd) -> Result<u16, Error<I2C::Error>> {
        let cmd_bytes = (cmd as u16).to_le_bytes();
        self.device
            .control()
            .write_async(|w| *w = field_sets::Control::from(cmd_bytes))
            .await
            .map_err(Error::I2c)?;
        let result = self
            .device
            .control()
            .read_async()
            .await
            .map_err(Error::I2c)?;
        let bytes: [u8; 2] = result.into();
        Ok(u16::from_le_bytes(bytes))
    }

    /// Send a control subcommand (write-only).
    pub async fn control_write(&mut self, cmd: ControlCmd) -> Result<(), Error<I2C::Error>> {
        let cmd_bytes = (cmd as u16).to_le_bytes();
        self.device
            .control()
            .write_async(|w| *w = field_sets::Control::from(cmd_bytes))
            .await
            .map_err(Error::I2c)
    }

    /// Read battery voltage in millivolts.
    pub async fn voltage(&mut self) -> Result<u16, Error<I2C::Error>> {
        let val = self
            .device
            .voltage()
            .read_async()
            .await
            .map_err(Error::I2c)?;
        let bytes: [u8; 2] = val.into();
        Ok(u16::from_le_bytes(bytes))
    }

    /// Read temperature in 0.1 Kelvin units.
    pub async fn temperature_raw(&mut self) -> Result<u16, Error<I2C::Error>> {
        let val = self
            .device
            .temperature()
            .read_async()
            .await
            .map_err(Error::I2c)?;
        let bytes: [u8; 2] = val.into();
        Ok(u16::from_le_bytes(bytes))
    }

    /// Read temperature in degrees Celsius.
    pub async fn temperature_celsius(&mut self) -> Result<f32, Error<I2C::Error>> {
        let raw = self.temperature_raw().await?;
        Ok((f32::from(raw) * 0.1) - 273.15)
    }

    /// Read state of charge (0-100%).
    pub async fn state_of_charge(&mut self) -> Result<u16, Error<I2C::Error>> {
        let val = self
            .device
            .state_of_charge()
            .read_async()
            .await
            .map_err(Error::I2c)?;
        let bytes: [u8; 2] = val.into();
        Ok(u16::from_le_bytes(bytes))
    }

    /// Read remaining capacity in mAh.
    pub async fn remaining_capacity(&mut self) -> Result<u16, Error<I2C::Error>> {
        let val = self
            .device
            .remaining_capacity()
            .read_async()
            .await
            .map_err(Error::I2c)?;
        let bytes: [u8; 2] = val.into();
        Ok(u16::from_le_bytes(bytes))
    }

    /// Read full charge capacity in mAh.
    pub async fn full_charge_capacity(&mut self) -> Result<u16, Error<I2C::Error>> {
        let val = self
            .device
            .full_charge_capacity()
            .read_async()
            .await
            .map_err(Error::I2c)?;
        let bytes: [u8; 2] = val.into();
        Ok(u16::from_le_bytes(bytes))
    }

    /// Read average current in mA (signed).
    pub async fn average_current(&mut self) -> Result<i16, Error<I2C::Error>> {
        let val = self
            .device
            .average_current()
            .read_async()
            .await
            .map_err(Error::I2c)?;
        let bytes: [u8; 2] = val.into();
        Ok(i16::from_le_bytes(bytes))
    }

    /// Read average power in mW (signed).
    pub async fn average_power(&mut self) -> Result<i16, Error<I2C::Error>> {
        let val = self
            .device
            .average_power()
            .read_async()
            .await
            .map_err(Error::I2c)?;
        let bytes: [u8; 2] = val.into();
        Ok(i16::from_le_bytes(bytes))
    }

    /// Read state of health percentage.
    pub async fn state_of_health(&mut self) -> Result<u8, Error<I2C::Error>> {
        let val = self
            .device
            .state_of_health()
            .read_async()
            .await
            .map_err(Error::I2c)?;
        let bytes: [u8; 2] = val.into();
        Ok(bytes[0])
    }

    /// Read status flags.
    pub async fn flags(&mut self) -> Result<field_sets::Flags, Error<I2C::Error>> {
        self.device.flags().read_async().await.map_err(Error::I2c)
    }

    /// Check if battery is detected.
    pub async fn is_battery_detected(&mut self) -> Result<bool, Error<I2C::Error>> {
        let flags = self.flags().await?;
        Ok(flags.bat_det())
    }

    /// Check if battery is charging.
    pub async fn is_charging(&mut self) -> Result<bool, Error<I2C::Error>> {
        let flags = self.flags().await?;
        Ok(flags.chg())
    }

    /// Check if battery is discharging.
    pub async fn is_discharging(&mut self) -> Result<bool, Error<I2C::Error>> {
        let flags = self.flags().await?;
        Ok(flags.dsg())
    }

    /// Check if battery is fully charged.
    pub async fn is_full_charged(&mut self) -> Result<bool, Error<I2C::Error>> {
        let flags = self.flags().await?;
        Ok(flags.fc())
    }

    /// Read firmware version.
    pub async fn firmware_version(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.control_read(ControlCmd::FwVersion).await
    }

    /// Read chemistry ID.
    pub async fn chemistry_id(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.control_read(ControlCmd::ChemId).await
    }

    /// Enter SEALED mode.
    pub async fn seal(&mut self) -> Result<(), Error<I2C::Error>> {
        self.control_write(ControlCmd::Sealed).await
    }

    /// Unseal the device.
    pub async fn unseal(&mut self) -> Result<(), Error<I2C::Error>> {
        let key_bytes = UNSEAL_KEY.to_le_bytes();
        // Send key first time
        self.device
            .control()
            .write_async(|w| *w = field_sets::Control::from(key_bytes))
            .await
            .map_err(Error::I2c)?;
        // Send key second time
        self.device
            .control()
            .write_async(|w| *w = field_sets::Control::from(key_bytes))
            .await
            .map_err(Error::I2c)
    }

    /// Enter config update mode.
    pub async fn enter_config_mode(&mut self) -> Result<(), Error<I2C::Error>> {
        self.control_write(ControlCmd::SetCfgUpdate).await
    }

    /// Exit config update mode with soft reset.
    pub async fn exit_config_mode(&mut self) -> Result<(), Error<I2C::Error>> {
        self.control_write(ControlCmd::SoftReset).await
    }

    /// Perform a soft reset.
    pub async fn soft_reset(&mut self) -> Result<(), Error<I2C::Error>> {
        self.control_write(ControlCmd::SoftReset).await
    }

    /// Set hibernate mode.
    pub async fn set_hibernate(&mut self) -> Result<(), Error<I2C::Error>> {
        self.control_write(ControlCmd::SetHibernate).await
    }

    /// Clear hibernate mode.
    pub async fn clear_hibernate(&mut self) -> Result<(), Error<I2C::Error>> {
        self.control_write(ControlCmd::ClearHibernate).await
    }

    /// Access the generated register API directly.
    pub fn device(&mut self) -> &mut Bq27441Device<DeviceInterfaceAsync<I2C>> {
        &mut self.device
    }

    /// Consume the driver and return the underlying I²C bus.
    pub fn destroy(self) -> I2C {
        self.device.interface.i2c
    }
}
