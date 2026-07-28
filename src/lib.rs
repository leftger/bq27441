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
use device_driver::AsyncBufferInterface;
#[cfg(feature = "async")]
use device_driver::AsyncRegisterInterface;
use device_driver::{BufferInterface, BufferInterfaceError, RegisterInterface};
use embedded_hal as hal;
#[cfg(feature = "async")]
use embedded_hal_async as hal_async;

#[allow(unsafe_code)]
#[allow(missing_docs)]
#[allow(clippy::doc_markdown, clippy::missing_errors_doc, clippy::identity_op)]
pub(crate) mod generated {
    device_driver::create_device!(
        device_name: Bq27441Device,
        manifest: "src/bq27441.yaml"
    );
}

pub use generated::{Bq27441Device, field_sets};

mod config;
#[cfg(feature = "async")]
mod config_async;
mod data_memory;
mod delay;
mod golden;
mod learning;

pub use config::{
    BatteryConfig, ChargeTerminationThresholds, ConfigExit, ConfigureOptions, ControlStatus,
    CurrentThresholds, DischargeThresholds, LearningPhase, OpConfig, OpConfigB, RaTable,
    SafetyThresholds, UpdateStatus,
};
pub use data_memory::{
    BLOCK_DATA_BASE, BLOCK_SIZE, BlockWriteOptions, CHECKSUM_SETTLE_MS, ConfigUpdateSession,
    DataMemoryBlockId, RA_TABLE_LEN, block_checksum, commit_block_checksum, data_memory_read_block,
    data_memory_read_subclass, data_memory_write_block, data_memory_write_block_with_options,
    data_memory_write_subclass, i16_field, i16_le, patch_checksum, select_block, subclass, u8_le,
    u16_field, u16_le,
};
#[cfg(feature = "async")]
pub use delay::DelayMsAsync;
pub use delay::{BusyWait, DelayMs};
pub use golden::{DataMemoryBlock, GoldenSnapshot, read_block, write_block};
pub use learning::{
    LearningProgress, learning_progress, qmax_mah, read_delta_voltage, read_qmax_cell_0,
    read_update_status,
};

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
    /// Device is sealed; unseal before data memory writes.
    Sealed,
    /// Not in CONFIG UPDATE mode.
    NotInConfigMode,
    /// CONFIG UPDATE mode transition timed out.
    ConfigModeTimeout,
    /// Block checksum readback did not match the value written.
    BlockChecksumMismatch,
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

/// State-of-health status byte (TRM §4.15, `StateOfHealth()` high byte).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SohStatus {
    /// SOH not valid (initialization).
    NotValid,
    /// Instant SOH value ready.
    InstantReady,
    /// Initial SOH value ready (default Qmax).
    InitialReady,
    /// SOH value ready (learned Qmax).
    Ready,
    /// Reserved value.
    Reserved(u8),
}

impl SohStatus {
    /// Decode the SOH status byte from `StateOfHealth()`.
    #[must_use]
    pub const fn from_byte(value: u8) -> Self {
        match value {
            0x00 => Self::NotValid,
            0x01 => Self::InstantReady,
            0x02 => Self::InitialReady,
            0x03 => Self::Ready,
            other => Self::Reserved(other),
        }
    }
}

fn u16_from_bytes<T: Into<[u8; 2]>>(value: T) -> u16 {
    u16::from_le_bytes(value.into())
}

fn i16_from_bytes<T: Into<[u8; 2]>>(value: T) -> i16 {
    i16::from_le_bytes(value.into())
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

impl<I2C> BufferInterfaceError for DeviceInterface<I2C>
where
    I2C: hal::i2c::I2c,
{
    type Error = I2C::Error;
}

#[cfg(feature = "async")]
impl<I2C> BufferInterfaceError for DeviceInterfaceAsync<I2C>
where
    I2C: hal_async::i2c::I2c,
{
    type Error = I2C::Error;
}

/// Read or write the block-data window one register byte at a time (TRM §5.5).
impl<I2C> BufferInterface for DeviceInterface<I2C>
where
    I2C: hal::i2c::I2c,
{
    type AddressType = u8;

    fn read(&mut self, address: Self::AddressType, buf: &mut [u8]) -> Result<usize, Self::Error> {
        for (index, slot) in buf.iter_mut().enumerate() {
            let mut byte = [0u8];
            let offset = u8::try_from(index).unwrap_or(u8::MAX);
            self.read_register(address.wrapping_add(offset), 8, &mut byte)?;
            *slot = byte[0];
        }
        Ok(buf.len())
    }

    fn write(&mut self, address: Self::AddressType, buf: &[u8]) -> Result<usize, Self::Error> {
        for (index, &byte) in buf.iter().enumerate() {
            let offset = u8::try_from(index).unwrap_or(u8::MAX);
            self.write_register(address.wrapping_add(offset), 8, &[byte])?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self, _address: Self::AddressType) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[cfg(feature = "async")]
impl<I2C> AsyncBufferInterface for DeviceInterfaceAsync<I2C>
where
    I2C: hal_async::i2c::I2c,
{
    type AddressType = u8;

    async fn read(
        &mut self,
        address: Self::AddressType,
        buf: &mut [u8],
    ) -> Result<usize, Self::Error> {
        for (index, slot) in buf.iter_mut().enumerate() {
            let mut byte = [0u8];
            let offset = u8::try_from(index).unwrap_or(u8::MAX);
            self.read_register(address.wrapping_add(offset), 8, &mut byte)
                .await?;
            *slot = byte[0];
        }
        Ok(buf.len())
    }

    async fn write(
        &mut self,
        address: Self::AddressType,
        buf: &[u8],
    ) -> Result<usize, Self::Error> {
        for (index, &byte) in buf.iter().enumerate() {
            let offset = u8::try_from(index).unwrap_or(u8::MAX);
            self.write_register(address.wrapping_add(offset), 8, &[byte])
                .await?;
        }
        Ok(buf.len())
    }

    async fn flush(&mut self, _address: Self::AddressType) -> Result<(), Self::Error> {
        Ok(())
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
        Ok(bytes[0])
    }

    /// Read state-of-health status (high byte of `StateOfHealth()`).
    pub fn state_of_health_status(&mut self) -> Result<SohStatus, Error<I2C::Error>> {
        let val = self.device.state_of_health().read().map_err(Error::I2c)?;
        let bytes: [u8; 2] = val.into();
        Ok(SohStatus::from_byte(bytes[1]))
    }

    /// Read uncompensated remaining capacity in mAh.
    pub fn nominal_available_capacity(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.device
            .nominal_available_capacity()
            .read()
            .map(u16_from_bytes)
            .map_err(Error::I2c)
    }

    /// Read uncompensated full capacity in mAh.
    pub fn full_available_capacity(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.device
            .full_available_capacity()
            .read()
            .map(u16_from_bytes)
            .map_err(Error::I2c)
    }

    /// Read standby current in mA (signed).
    pub fn standby_current(&mut self) -> Result<i16, Error<I2C::Error>> {
        self.device
            .standby_current()
            .read()
            .map(i16_from_bytes)
            .map_err(Error::I2c)
    }

    /// Read maximum load current in mA (signed).
    pub fn max_load_current(&mut self) -> Result<i16, Error<I2C::Error>> {
        self.device
            .max_load_current()
            .read()
            .map(i16_from_bytes)
            .map_err(Error::I2c)
    }

    /// Read internal temperature in 0.1 Kelvin units.
    pub fn internal_temperature_raw(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.device
            .internal_temperature()
            .read()
            .map(u16_from_bytes)
            .map_err(Error::I2c)
    }

    /// Read internal temperature in degrees Celsius.
    pub fn internal_temperature_celsius(&mut self) -> Result<f32, Error<I2C::Error>> {
        let raw = self.internal_temperature_raw()?;
        Ok((f32::from(raw) * 0.1) - 273.15)
    }

    /// Read unfiltered remaining capacity in mAh.
    pub fn remaining_capacity_unfiltered(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.device
            .remaining_capacity_unfiltered()
            .read()
            .map(u16_from_bytes)
            .map_err(Error::I2c)
    }

    /// Read filtered remaining capacity in mAh.
    pub fn remaining_capacity_filtered(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.device
            .remaining_capacity_filtered()
            .read()
            .map(u16_from_bytes)
            .map_err(Error::I2c)
    }

    /// Read unfiltered full charge capacity in mAh.
    pub fn full_charge_capacity_unfiltered(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.device
            .full_charge_capacity_unfiltered()
            .read()
            .map(u16_from_bytes)
            .map_err(Error::I2c)
    }

    /// Read filtered full charge capacity in mAh.
    pub fn full_charge_capacity_filtered(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.device
            .full_charge_capacity_filtered()
            .read()
            .map(u16_from_bytes)
            .map_err(Error::I2c)
    }

    /// Read unfiltered state of charge (0–100%).
    pub fn state_of_charge_unfiltered(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.device
            .state_of_charge_unfiltered()
            .read()
            .map(u16_from_bytes)
            .map_err(Error::I2c)
    }

    /// Set temperature in 0.1 Kelvin units when `OpConfig[TEMPS]=1`.
    pub fn set_temperature_raw(&mut self, raw_deci_k: u16) -> Result<(), Error<I2C::Error>> {
        let bytes = raw_deci_k.to_le_bytes();
        self.device
            .temperature()
            .write(|register| *register = field_sets::Temperature::from(bytes))
            .map_err(Error::I2c)
    }

    /// Set host-provided temperature in degrees Celsius when `OpConfig[TEMPS]=1`.
    // Rust's `as` cast from float to integer saturates at the type's bounds and
    // maps NaN to 0, so out-of-range input clamps rather than wrapping.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn set_temperature_celsius(&mut self, celsius: f32) -> Result<(), Error<I2C::Error>> {
        let raw = ((celsius + 273.15) * 10.0) as u16;
        self.set_temperature_raw(raw)
    }

    /// Read data memory code.
    pub fn dm_code(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.control_read(ControlCmd::DmCode)
    }

    /// Read previous MAC write subcommand.
    pub fn prev_mac_write(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.control_read(ControlCmd::PrevMacWrite)
    }

    /// Signal battery insertion when `OpConfig[BIE]=0`.
    pub fn bat_insert(&mut self) -> Result<(), Error<I2C::Error>> {
        self.control_write(ControlCmd::BatInsert)
    }

    /// Signal battery removal when `OpConfig[BIE]=0`.
    pub fn bat_remove(&mut self) -> Result<(), Error<I2C::Error>> {
        self.control_write(ControlCmd::BatRemove)
    }

    /// Enable shutdown mode entry.
    pub fn shutdown_enable(&mut self) -> Result<(), Error<I2C::Error>> {
        self.control_write(ControlCmd::ShutdownEnable)
    }

    /// Enter shutdown mode (requires prior [`Self::shutdown_enable`]).
    pub fn shutdown(&mut self) -> Result<(), Error<I2C::Error>> {
        self.control_write(ControlCmd::Shutdown)
    }

    /// Pulse the GPOUT alert pin.
    pub fn pulse_gpout(&mut self) -> Result<(), Error<I2C::Error>> {
        self.control_write(ControlCmd::PulseGpout)
    }

    /// Perform a full device reset with OCV measurement (`Control 0x0041`).
    pub fn reset(&mut self) -> Result<(), Error<I2C::Error>> {
        self.control_write(ControlCmd::Reset)
    }

    /// Return `true` when a POR or reset occurred (`Flags()[ITPOR]`).
    pub fn needs_config_reload(&mut self) -> Result<bool, Error<I2C::Error>> {
        Ok(self.flags()?.itpor())
    }

    /// Return `true` when over-temperature is detected.
    pub fn is_over_temp(&mut self) -> Result<bool, Error<I2C::Error>> {
        Ok(self.flags()?.ot())
    }

    /// Return `true` when under-temperature is detected.
    pub fn is_under_temp(&mut self) -> Result<bool, Error<I2C::Error>> {
        Ok(self.flags()?.ut())
    }

    /// Poll learning-cycle progress (Update Status when unsealed).
    pub fn learning_progress(&mut self) -> Result<LearningProgress, Error<I2C::Error>> {
        let status = self.control_status()?;
        let sealed = status.sealed();
        learning::learning_progress(
            &mut self.device,
            status,
            |device| device.flags().read().map_err(Error::I2c),
            sealed,
        )
    }

    /// Read Update Status from data memory (device must be unsealed).
    pub fn update_status(&mut self) -> Result<UpdateStatus, Error<I2C::Error>> {
        if self.is_sealed()? {
            return Err(Error::Sealed);
        }
        learning::read_update_status(&mut self.device)
    }

    /// Read normalized Qmax Cell 0 (device must be unsealed).
    pub fn qmax_cell_0(&mut self) -> Result<i16, Error<I2C::Error>> {
        if self.is_sealed()? {
            return Err(Error::Sealed);
        }
        learning::read_qmax_cell_0(&mut self.device)
    }

    /// Read learned Delta Voltage in mV (device must be unsealed).
    pub fn delta_voltage_mv(&mut self) -> Result<i16, Error<I2C::Error>> {
        if self.is_sealed()? {
            return Err(Error::Sealed);
        }
        learning::read_delta_voltage(&mut self.device)
    }

    /// Read a golden-file snapshot from data memory (device must be unsealed).
    pub fn read_golden_snapshot(&mut self) -> Result<GoldenSnapshot, Error<I2C::Error>> {
        if self.is_sealed()? {
            return Err(Error::Sealed);
        }
        GoldenSnapshot::read(&mut self.device)
    }

    /// Write a golden-file snapshot in CONFIG UPDATE mode.
    pub fn write_golden_snapshot<D: DelayMs>(
        &mut self,
        snapshot: GoldenSnapshot,
        delay: &mut D,
    ) -> Result<(), Error<I2C::Error>> {
        let flags = self.device.flags().read().map_err(Error::I2c)?;
        if !flags.cfgupmode() {
            return Err(Error::NotInConfigMode);
        }
        snapshot.write(&mut self.device, delay)
    }

    /// Read an arbitrary data memory block (device must be unsealed).
    pub fn read_data_memory_block(
        &mut self,
        subclass_id: u8,
        block_index: u8,
    ) -> Result<DataMemoryBlock, Error<I2C::Error>> {
        if self.is_sealed()? {
            return Err(Error::Sealed);
        }
        golden::read_block(&mut self.device, subclass_id, block_index)
    }

    /// Write an arbitrary data memory block in CONFIG UPDATE mode.
    ///
    /// Commits the block checksum and verifies readback after [`CHECKSUM_SETTLE_MS`].
    pub fn write_data_memory_block<D: DelayMs>(
        &mut self,
        block: &DataMemoryBlock,
        delay: &mut D,
    ) -> Result<(), Error<I2C::Error>> {
        let flags = self.device.flags().read().map_err(Error::I2c)?;
        if !flags.cfgupmode() {
            return Err(Error::NotInConfigMode);
        }
        data_memory_write_block(
            &mut self.device,
            block.subclass_id,
            block.block_index,
            &block.data,
            delay,
        )
    }

    /// Write an arbitrary data memory block with custom write options.
    pub fn write_data_memory_block_with_options<D: DelayMs>(
        &mut self,
        block: &DataMemoryBlock,
        options: BlockWriteOptions,
        delay: &mut D,
    ) -> Result<(), Error<I2C::Error>> {
        let flags = self.device.flags().read().map_err(Error::I2c)?;
        if !flags.cfgupmode() {
            return Err(Error::NotInConfigMode);
        }
        data_memory_write_block_with_options(
            &mut self.device,
            block.subclass_id,
            block.block_index,
            &block.data,
            options,
            delay,
        )
    }

    /// Read consecutive blocks from a data memory subclass (device must be unsealed).
    ///
    /// `data.len()` must be a non-zero multiple of [`BLOCK_SIZE`].
    pub fn read_data_memory_subclass(
        &mut self,
        subclass_id: u8,
        data: &mut [u8],
    ) -> Result<(), Error<I2C::Error>> {
        if self.is_sealed()? {
            return Err(Error::Sealed);
        }
        data_memory_read_subclass(&mut self.device, subclass_id, data)
    }

    /// Write consecutive blocks to a data memory subclass in CONFIG UPDATE mode.
    ///
    /// `data.len()` must be a non-zero multiple of [`BLOCK_SIZE`].
    pub fn write_data_memory_subclass<D: DelayMs>(
        &mut self,
        subclass_id: u8,
        data: &[u8],
        options: BlockWriteOptions,
        delay: &mut D,
    ) -> Result<(), Error<I2C::Error>> {
        let flags = self.device.flags().read().map_err(Error::I2c)?;
        if !flags.cfgupmode() {
            return Err(Error::NotInConfigMode);
        }
        data_memory_write_subclass(&mut self.device, subclass_id, data, options, delay)
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

    /// Exit config update mode with resimulation (updates SOC).
    pub fn exit_config_resim(&mut self) -> Result<(), Error<I2C::Error>> {
        self.control_write(ControlCmd::ExitResim)
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

    /// Read state-of-health status (high byte of `StateOfHealth()`).
    pub async fn state_of_health_status(&mut self) -> Result<SohStatus, Error<I2C::Error>> {
        let val = self
            .device
            .state_of_health()
            .read_async()
            .await
            .map_err(Error::I2c)?;
        let bytes: [u8; 2] = val.into();
        Ok(SohStatus::from_byte(bytes[1]))
    }

    /// Read uncompensated remaining capacity in mAh.
    pub async fn nominal_available_capacity(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.device
            .nominal_available_capacity()
            .read_async()
            .await
            .map(u16_from_bytes)
            .map_err(Error::I2c)
    }

    /// Read uncompensated full capacity in mAh.
    pub async fn full_available_capacity(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.device
            .full_available_capacity()
            .read_async()
            .await
            .map(u16_from_bytes)
            .map_err(Error::I2c)
    }

    /// Read standby current in mA (signed).
    pub async fn standby_current(&mut self) -> Result<i16, Error<I2C::Error>> {
        self.device
            .standby_current()
            .read_async()
            .await
            .map(i16_from_bytes)
            .map_err(Error::I2c)
    }

    /// Read maximum load current in mA (signed).
    pub async fn max_load_current(&mut self) -> Result<i16, Error<I2C::Error>> {
        self.device
            .max_load_current()
            .read_async()
            .await
            .map(i16_from_bytes)
            .map_err(Error::I2c)
    }

    /// Read internal temperature in 0.1 Kelvin units.
    pub async fn internal_temperature_raw(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.device
            .internal_temperature()
            .read_async()
            .await
            .map(u16_from_bytes)
            .map_err(Error::I2c)
    }

    /// Read internal temperature in degrees Celsius.
    pub async fn internal_temperature_celsius(&mut self) -> Result<f32, Error<I2C::Error>> {
        let raw = self.internal_temperature_raw().await?;
        Ok((f32::from(raw) * 0.1) - 273.15)
    }

    /// Read unfiltered remaining capacity in mAh.
    pub async fn remaining_capacity_unfiltered(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.device
            .remaining_capacity_unfiltered()
            .read_async()
            .await
            .map(u16_from_bytes)
            .map_err(Error::I2c)
    }

    /// Read filtered remaining capacity in mAh.
    pub async fn remaining_capacity_filtered(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.device
            .remaining_capacity_filtered()
            .read_async()
            .await
            .map(u16_from_bytes)
            .map_err(Error::I2c)
    }

    /// Read unfiltered full charge capacity in mAh.
    pub async fn full_charge_capacity_unfiltered(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.device
            .full_charge_capacity_unfiltered()
            .read_async()
            .await
            .map(u16_from_bytes)
            .map_err(Error::I2c)
    }

    /// Read filtered full charge capacity in mAh.
    pub async fn full_charge_capacity_filtered(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.device
            .full_charge_capacity_filtered()
            .read_async()
            .await
            .map(u16_from_bytes)
            .map_err(Error::I2c)
    }

    /// Read unfiltered state of charge (0–100%).
    pub async fn state_of_charge_unfiltered(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.device
            .state_of_charge_unfiltered()
            .read_async()
            .await
            .map(u16_from_bytes)
            .map_err(Error::I2c)
    }

    /// Set temperature in 0.1 Kelvin units when `OpConfig[TEMPS]=1`.
    pub async fn set_temperature_raw(&mut self, raw_deci_k: u16) -> Result<(), Error<I2C::Error>> {
        let bytes = raw_deci_k.to_le_bytes();
        self.device
            .temperature()
            .write_async(|register| *register = field_sets::Temperature::from(bytes))
            .await
            .map_err(Error::I2c)
    }

    /// Set host-provided temperature in degrees Celsius when `OpConfig[TEMPS]=1`.
    // Rust's `as` cast from float to integer saturates at the type's bounds and
    // maps NaN to 0, so out-of-range input clamps rather than wrapping.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub async fn set_temperature_celsius(&mut self, celsius: f32) -> Result<(), Error<I2C::Error>> {
        let raw = ((celsius + 273.15) * 10.0) as u16;
        self.set_temperature_raw(raw).await
    }

    /// Read data memory code.
    pub async fn dm_code(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.control_read(ControlCmd::DmCode).await
    }

    /// Read previous MAC write subcommand.
    pub async fn prev_mac_write(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.control_read(ControlCmd::PrevMacWrite).await
    }

    /// Signal battery insertion when `OpConfig[BIE]=0`.
    pub async fn bat_insert(&mut self) -> Result<(), Error<I2C::Error>> {
        self.control_write(ControlCmd::BatInsert).await
    }

    /// Signal battery removal when `OpConfig[BIE]=0`.
    pub async fn bat_remove(&mut self) -> Result<(), Error<I2C::Error>> {
        self.control_write(ControlCmd::BatRemove).await
    }

    /// Enable shutdown mode entry.
    pub async fn shutdown_enable(&mut self) -> Result<(), Error<I2C::Error>> {
        self.control_write(ControlCmd::ShutdownEnable).await
    }

    /// Enter shutdown mode (requires prior [`Self::shutdown_enable`]).
    pub async fn shutdown(&mut self) -> Result<(), Error<I2C::Error>> {
        self.control_write(ControlCmd::Shutdown).await
    }

    /// Pulse the GPOUT alert pin.
    pub async fn pulse_gpout(&mut self) -> Result<(), Error<I2C::Error>> {
        self.control_write(ControlCmd::PulseGpout).await
    }

    /// Perform a full device reset with OCV measurement (`Control 0x0041`).
    pub async fn reset(&mut self) -> Result<(), Error<I2C::Error>> {
        self.control_write(ControlCmd::Reset).await
    }

    /// Return `true` when a POR or reset occurred (`Flags()[ITPOR]`).
    pub async fn needs_config_reload(&mut self) -> Result<bool, Error<I2C::Error>> {
        Ok(self.flags().await?.itpor())
    }

    /// Return `true` when over-temperature is detected.
    pub async fn is_over_temp(&mut self) -> Result<bool, Error<I2C::Error>> {
        Ok(self.flags().await?.ot())
    }

    /// Return `true` when under-temperature is detected.
    pub async fn is_under_temp(&mut self) -> Result<bool, Error<I2C::Error>> {
        Ok(self.flags().await?.ut())
    }

    /// Poll learning-cycle progress (Update Status when unsealed).
    pub async fn learning_progress(&mut self) -> Result<LearningProgress, Error<I2C::Error>> {
        let status = self.control_status().await?;
        let sealed = status.sealed();
        let flags = self.flags().await?;
        learning::learning_progress_async(&mut self.device, status, flags, sealed).await
    }

    /// Read Update Status from data memory (device must be unsealed).
    pub async fn update_status(&mut self) -> Result<UpdateStatus, Error<I2C::Error>> {
        if self.is_sealed().await? {
            return Err(Error::Sealed);
        }
        learning::read_update_status_async(&mut self.device).await
    }

    /// Read normalized Qmax Cell 0 (device must be unsealed).
    pub async fn qmax_cell_0(&mut self) -> Result<i16, Error<I2C::Error>> {
        if self.is_sealed().await? {
            return Err(Error::Sealed);
        }
        learning::read_qmax_cell_0_async(&mut self.device).await
    }

    /// Read learned Delta Voltage in mV (device must be unsealed).
    pub async fn delta_voltage_mv(&mut self) -> Result<i16, Error<I2C::Error>> {
        if self.is_sealed().await? {
            return Err(Error::Sealed);
        }
        learning::read_delta_voltage_async(&mut self.device).await
    }

    /// Read a golden-file snapshot from data memory (device must be unsealed).
    pub async fn read_golden_snapshot(&mut self) -> Result<GoldenSnapshot, Error<I2C::Error>> {
        if self.is_sealed().await? {
            return Err(Error::Sealed);
        }
        GoldenSnapshot::read_async(&mut self.device).await
    }

    /// Write a golden-file snapshot in CONFIG UPDATE mode.
    pub async fn write_golden_snapshot<D: DelayMsAsync>(
        &mut self,
        snapshot: GoldenSnapshot,
        delay: &mut D,
    ) -> Result<(), Error<I2C::Error>> {
        let flags = self.device.flags().read_async().await.map_err(Error::I2c)?;
        if !flags.cfgupmode() {
            return Err(Error::NotInConfigMode);
        }
        snapshot.write_async(&mut self.device, delay).await
    }

    /// Read an arbitrary data memory block (device must be unsealed).
    pub async fn read_data_memory_block(
        &mut self,
        subclass_id: u8,
        block_index: u8,
    ) -> Result<DataMemoryBlock, Error<I2C::Error>> {
        if self.is_sealed().await? {
            return Err(Error::Sealed);
        }
        golden::read_block_async(&mut self.device, subclass_id, block_index).await
    }

    /// Write an arbitrary data memory block in CONFIG UPDATE mode.
    ///
    /// Commits the block checksum and verifies readback after [`CHECKSUM_SETTLE_MS`].
    pub async fn write_data_memory_block<D: DelayMsAsync>(
        &mut self,
        block: &DataMemoryBlock,
        delay: &mut D,
    ) -> Result<(), Error<I2C::Error>> {
        let flags = self.device.flags().read_async().await.map_err(Error::I2c)?;
        if !flags.cfgupmode() {
            return Err(Error::NotInConfigMode);
        }
        data_memory::data_memory_write_block_async(
            &mut self.device,
            block.subclass_id,
            block.block_index,
            &block.data,
            delay,
        )
        .await
    }

    /// Write an arbitrary data memory block with custom write options.
    pub async fn write_data_memory_block_with_options<D: DelayMsAsync>(
        &mut self,
        block: &DataMemoryBlock,
        options: BlockWriteOptions,
        delay: &mut D,
    ) -> Result<(), Error<I2C::Error>> {
        let flags = self.device.flags().read_async().await.map_err(Error::I2c)?;
        if !flags.cfgupmode() {
            return Err(Error::NotInConfigMode);
        }
        data_memory::data_memory_write_block_with_options_async(
            &mut self.device,
            block.subclass_id,
            block.block_index,
            &block.data,
            options,
            delay,
        )
        .await
    }

    /// Read consecutive blocks from a data memory subclass (device must be unsealed).
    ///
    /// `data.len()` must be a non-zero multiple of [`BLOCK_SIZE`].
    pub async fn read_data_memory_subclass(
        &mut self,
        subclass_id: u8,
        data: &mut [u8],
    ) -> Result<(), Error<I2C::Error>> {
        if self.is_sealed().await? {
            return Err(Error::Sealed);
        }
        data_memory::data_memory_read_subclass_async(&mut self.device, subclass_id, data).await
    }

    /// Write consecutive blocks to a data memory subclass in CONFIG UPDATE mode.
    ///
    /// `data.len()` must be a non-zero multiple of [`BLOCK_SIZE`].
    pub async fn write_data_memory_subclass<D: DelayMsAsync>(
        &mut self,
        subclass_id: u8,
        data: &[u8],
        options: BlockWriteOptions,
        delay: &mut D,
    ) -> Result<(), Error<I2C::Error>> {
        let flags = self.device.flags().read_async().await.map_err(Error::I2c)?;
        if !flags.cfgupmode() {
            return Err(Error::NotInConfigMode);
        }
        data_memory::data_memory_write_subclass_async(
            &mut self.device,
            subclass_id,
            data,
            options,
            delay,
        )
        .await
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

    /// Exit config update mode with resimulation (updates SOC).
    pub async fn exit_config_resim(&mut self) -> Result<(), Error<I2C::Error>> {
        self.control_write(ControlCmd::ExitResim).await
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
