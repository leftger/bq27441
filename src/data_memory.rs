//! Data Memory block transfer protocol (TRM §5.3–5.7, §6.1.1).
//!
//! Field layouts are generated from [`crate::bq27441.yaml`] as `DM_*_BLOCK*` blocks at
//! `0x40`. Call [`select_block`] before using generated layout accessors.

use core::fmt::Debug;

use device_driver::RegisterInterface;
use device_driver::embedded_io;

use crate::delay::DelayMs;
use crate::generated::{Bq27441Device, field_sets};
use crate::Error;

/// Delay after writing a block checksum before readback verification (SLUA801 §4.2).
pub const CHECKSUM_SETTLE_MS: u32 = 10;

/// Options for data memory block writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockWriteOptions {
    /// When `true`, wait [`CHECKSUM_SETTLE_MS`] then verify the checksum readback.
    pub verify_checksum: bool,
}

impl BlockWriteOptions {
    /// Skip checksum readback verification (for callers that manage timing themselves).
    #[must_use]
    pub const fn unverified() -> Self {
        Self {
            verify_checksum: false,
        }
    }
}

impl Default for BlockWriteOptions {
    fn default() -> Self {
        Self {
            verify_checksum: true,
        }
    }
}

/// Size of a Data Memory transfer block in bytes.
pub const BLOCK_SIZE: usize = 32;

/// I²C address of the first `BlockData()` byte.
pub const BLOCK_DATA_BASE: u8 = 0x40;

/// Data Memory subclass identifiers (TRM Tables 6-2–6-6).
pub mod subclass {
    /// Safety thresholds subclass.
    pub const SAFETY: u8 = 2;
    /// Charge termination thresholds subclass.
    pub const CHARGE_TERMINATION: u8 = 36;
    /// SOC alert thresholds subclass.
    pub const DISCHARGE: u8 = 49;
    /// OpConfig / OpConfigB subclass.
    pub const REGISTERS: u8 = 64;
    /// Current threshold subclass.
    pub const CURRENT_THRESHOLDS: u8 = 81;
    /// Design capacity, voltages, and gauging state subclass.
    pub const STATE: u8 = 82;
    /// Ra table RAM subclass.
    pub const RA_RAM: u8 = 89;
}

/// Number of Ra table entries in subclass 89.
pub const RA_TABLE_LEN: usize = 15;

/// Decode a generated 16-bit field set as signed little-endian.
#[inline]
#[must_use]
pub fn i16_le<T: Into<[u8; 2]>>(value: T) -> i16 {
    i16::from_le_bytes(value.into())
}

/// Decode a generated 16-bit field set as unsigned little-endian.
#[inline]
#[must_use]
pub fn u16_le<T: Into<[u8; 2]>>(value: T) -> u16 {
    u16::from_le_bytes(value.into())
}

/// Decode a generated 8-bit field set.
#[inline]
#[must_use]
pub fn u8_le<T: Into<[u8; 1]>>(value: T) -> u8 {
    value.into()[0]
}

/// Encode a signed 16-bit value for a generated field set write.
#[inline]
#[must_use]
pub fn i16_field(value: i16) -> [u8; 2] {
    value.to_le_bytes()
}

/// Encode an unsigned 16-bit value for a generated field set write.
#[inline]
#[must_use]
pub fn u16_field(value: u16) -> [u8; 2] {
    value.to_le_bytes()
}

/// Compute the block checksum: `255 − (sum of 32 bytes mod 256)`.
#[must_use]
pub fn block_checksum(data: &[u8; BLOCK_SIZE]) -> u8 {
    let sum: u16 = data.iter().map(|&byte| u16::from(byte)).sum();
    255 - (sum as u8)
}

/// Incrementally update a checksum after replacing bytes (TRM §3.1 step 10).
#[must_use]
pub fn patch_checksum(old_checksum: u8, old_bytes: &[u8], new_bytes: &[u8]) -> u8 {
    debug_assert_eq!(old_bytes.len(), new_bytes.len());

    let mut temp = u16::from(255u8.wrapping_sub(old_checksum));
    for &byte in old_bytes {
        temp = temp.wrapping_sub(u16::from(byte));
    }
    temp %= 256;

    let mut new_sum = temp;
    for &byte in new_bytes {
        new_sum = new_sum.wrapping_add(u16::from(byte));
    }
    new_sum %= 256;

    255u8.wrapping_sub(new_sum as u8)
}

fn map_i2c<E: Debug>(err: E) -> Error<E> {
    Error::I2c(err)
}

fn map_read_exact<E: Debug>(err: embedded_io::ReadExactError<E>) -> Error<E> {
    match err {
        embedded_io::ReadExactError::UnexpectedEof => Error::InvalidParam,
        embedded_io::ReadExactError::Other(err) => Error::I2c(err),
    }
}

/// Select a data memory subclass block for access through `0x40`–`0x5F`.
pub fn select_block<I, E>(
    device: &mut Bq27441Device<I>,
    subclass_id: u8,
    block_index: u8,
) -> Result<(), Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    device
        .block_data_control()
        .write(|register| *register = field_sets::BlockDataControl::from([0u8]))
        .map_err(map_i2c)?;

    device
        .data_class()
        .write(|register| *register = field_sets::DataClass::from([subclass_id]))
        .map_err(map_i2c)?;

    device
        .data_block()
        .write(|register| *register = field_sets::DataBlock::from([block_index]))
        .map_err(map_i2c)?;

    Ok(())
}

fn read_block_checksum<I, E>(device: &mut Bq27441Device<I>) -> Result<u8, Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    Ok(u8_le(
        device
            .block_data_checksum()
            .read()
            .map_err(map_i2c)?,
    ))
}

/// Identifies a 32-byte data memory block within a subclass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataMemoryBlockId {
    /// Data memory subclass ID.
    pub subclass_id: u8,
    /// Block index within the subclass (0–31).
    pub block_index: u8,
}

/// Tracks data memory blocks modified during a config update session.
///
/// Write fields with [`select_block`] and the generated layout accessors, call
/// [`Self::mark_block`] for each touched block, then [`Self::commit_all`] once
/// before exiting CONFIG UPDATE mode (SLUA801 / constellation `userConfigControl`
/// pattern).
#[derive(Debug, Default)]
pub struct ConfigUpdateSession {
    pending: [Option<DataMemoryBlockId>; Self::CAPACITY],
    len: usize,
}

impl ConfigUpdateSession {
    const CAPACITY: usize = 16;

    /// Create an empty session.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pending: [None; Self::CAPACITY],
            len: 0,
        }
    }

    /// Record that `subclass_id` / `block_index` was modified and needs a checksum.
    pub fn mark_block(&mut self, subclass_id: u8, block_index: u8) {
        if self
            .pending
            .iter()
            .flatten()
            .any(|block| block.subclass_id == subclass_id && block.block_index == block_index)
        {
            return;
        }

        if self.len >= Self::CAPACITY {
            return;
        }

        self.pending[self.len] = Some(DataMemoryBlockId {
            subclass_id,
            block_index,
        });
        self.len += 1;
    }

    /// Number of blocks waiting for checksum commit.
    #[must_use]
    pub const fn pending_count(&self) -> usize {
        self.len
    }

    /// Commit checksums for every marked block (re-selects each block first).
    pub fn commit_all<I, E>(self, device: &mut Bq27441Device<I>) -> Result<(), Error<E>>
    where
        I: RegisterInterface<AddressType = u8, Error = E>
            + device_driver::BufferInterface<AddressType = u8, Error = E>,
        E: Debug,
    {
        for block in self.pending.into_iter().flatten() {
            select_block(device, block.subclass_id, block.block_index)?;
            commit_block_checksum(device)?;
        }
        Ok(())
    }
}

fn write_block_checksum<I, E>(device: &mut Bq27441Device<I>, checksum: u8) -> Result<(), Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    device
        .block_data_checksum()
        .write(|register| *register = field_sets::BlockDataChecksum::from([checksum]))
        .map_err(map_i2c)
}

fn validate_subclass_len(len: usize) -> Result<(), Error<()>> {
    if len == 0 || !len.is_multiple_of(BLOCK_SIZE) {
        return Err(Error::InvalidParam);
    }
    Ok(())
}

fn write_block_committed<I, E, D>(
    device: &mut Bq27441Device<I>,
    subclass_id: u8,
    block_index: u8,
    data: &[u8; BLOCK_SIZE],
    options: BlockWriteOptions,
    delay: &mut D,
) -> Result<(), Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E> + device_driver::BufferInterface<AddressType = u8, Error = E>,
    E: Debug,
    D: DelayMs,
{
    select_block(device, subclass_id, block_index)?;
    write_block_raw(device, data)?;
    let checksum = block_checksum(data);
    write_block_checksum(device, checksum)?;

    if options.verify_checksum {
        delay.delay_ms(CHECKSUM_SETTLE_MS);
        select_block(device, subclass_id, block_index)?;
        if read_block_checksum(device)? != checksum {
            return Err(Error::BlockChecksumMismatch);
        }
    }

    Ok(())
}

fn read_block_raw<I, E>(device: &mut Bq27441Device<I>) -> Result<[u8; BLOCK_SIZE], Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E> + device_driver::BufferInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    let mut block = [0u8; BLOCK_SIZE];
    device
        .block_data()
        .read_exact(&mut block)
        .map_err(map_read_exact)?;
    Ok(block)
}

fn write_block_raw<I, E>(
    device: &mut Bq27441Device<I>,
    data: &[u8; BLOCK_SIZE],
) -> Result<(), Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E> + device_driver::BufferInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    device
        .block_data()
        .write_all(data)
        .map_err(map_i2c)
}

/// Read a 32-byte Data Memory block.
pub fn data_memory_read_block<I, E>(
    device: &mut Bq27441Device<I>,
    subclass_id: u8,
    block_index: u8,
) -> Result<[u8; BLOCK_SIZE], Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E> + device_driver::BufferInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    select_block(device, subclass_id, block_index)?;
    read_block_raw(device)
}

/// Write a 32-byte Data Memory block, commit the checksum, and verify readback.
///
/// Waits [`CHECKSUM_SETTLE_MS`] on `delay` before reading back the checksum.
pub fn data_memory_write_block<I, E, D>(
    device: &mut Bq27441Device<I>,
    subclass_id: u8,
    block_index: u8,
    data: &[u8; BLOCK_SIZE],
    delay: &mut D,
) -> Result<(), Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E> + device_driver::BufferInterface<AddressType = u8, Error = E>,
    E: Debug,
    D: DelayMs,
{
    data_memory_write_block_with_options(
        device,
        subclass_id,
        block_index,
        data,
        BlockWriteOptions::default(),
        delay,
    )
}

/// Write a 32-byte Data Memory block with configurable checksum verification.
pub fn data_memory_write_block_with_options<I, E, D>(
    device: &mut Bq27441Device<I>,
    subclass_id: u8,
    block_index: u8,
    data: &[u8; BLOCK_SIZE],
    options: BlockWriteOptions,
    delay: &mut D,
) -> Result<(), Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E> + device_driver::BufferInterface<AddressType = u8, Error = E>,
    E: Debug,
    D: DelayMs,
{
    write_block_committed(device, subclass_id, block_index, data, options, delay)
}

/// Read consecutive 32-byte blocks belonging to a data memory subclass.
///
/// `data.len()` must be a non-zero multiple of [`BLOCK_SIZE`].
pub fn data_memory_read_subclass<I, E>(
    device: &mut Bq27441Device<I>,
    subclass_id: u8,
    data: &mut [u8],
) -> Result<(), Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E> + device_driver::BufferInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    validate_subclass_len(data.len()).map_err(|_| Error::InvalidParam)?;

    for (block_index, chunk) in data.chunks_mut(BLOCK_SIZE).enumerate() {
        let block = data_memory_read_block(device, subclass_id, block_index as u8)?;
        chunk.copy_from_slice(&block);
    }

    Ok(())
}

/// Write consecutive 32-byte blocks belonging to a data memory subclass.
///
/// `data.len()` must be a non-zero multiple of [`BLOCK_SIZE`].
pub fn data_memory_write_subclass<I, E, D>(
    device: &mut Bq27441Device<I>,
    subclass_id: u8,
    data: &[u8],
    options: BlockWriteOptions,
    delay: &mut D,
) -> Result<(), Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E> + device_driver::BufferInterface<AddressType = u8, Error = E>,
    E: Debug,
    D: DelayMs,
{
    validate_subclass_len(data.len()).map_err(|_| Error::InvalidParam)?;

    for (block_index, chunk) in data.chunks(BLOCK_SIZE).enumerate() {
        let mut block = [0u8; BLOCK_SIZE];
        block.copy_from_slice(chunk);
        write_block_committed(
            device,
            subclass_id,
            block_index as u8,
            &block,
            options,
            delay,
        )?;
    }

    Ok(())
}

/// Recompute and write the checksum for the currently selected block.
pub fn commit_block_checksum<I, E>(device: &mut Bq27441Device<I>) -> Result<(), Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E> + device_driver::BufferInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    let block = read_block_raw(device)?;
    write_block_checksum(device, block_checksum(&block))
}

/// Read `OpConfig` from the extended command shortcut (available when sealed).
pub fn read_opconfig<I, E>(device: &mut Bq27441Device<I>) -> Result<field_sets::Opconfig, Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    device.opconfig().read().map_err(map_i2c)
}

/// Read design capacity from the extended command shortcut (available when sealed).
pub fn read_design_capacity<I, E>(device: &mut Bq27441Device<I>) -> Result<u16, Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    Ok(u16_le(device.design_capacity().read().map_err(map_i2c)?))
}

#[cfg(feature = "async")]
mod async_ops {
    use super::*;
    use crate::delay::DelayMsAsync;
    use device_driver::{AsyncBufferInterface, AsyncRegisterInterface};

    async fn read_block_checksum<I, E>(device: &mut Bq27441Device<I>) -> Result<u8, Error<E>>
    where
        I: AsyncRegisterInterface<AddressType = u8, Error = E>,
        E: Debug,
    {
        Ok(u8_le(
            device
                .block_data_checksum()
                .read_async()
                .await
                .map_err(map_i2c)?,
        ))
    }

    async fn write_block_committed<I, E, D>(
        device: &mut Bq27441Device<I>,
        subclass_id: u8,
        block_index: u8,
        data: &[u8; BLOCK_SIZE],
        options: BlockWriteOptions,
        delay: &mut D,
    ) -> Result<(), Error<E>>
    where
        I: AsyncRegisterInterface<AddressType = u8, Error = E> + AsyncBufferInterface<AddressType = u8, Error = E>,
        E: Debug,
        D: DelayMsAsync,
    {
        select_block(device, subclass_id, block_index).await?;
        device
            .block_data()
            .write_all_async(data)
            .await
            .map_err(map_i2c)?;
        let checksum = block_checksum(data);
        write_block_checksum(device, checksum).await?;

        if options.verify_checksum {
            delay.delay_ms(CHECKSUM_SETTLE_MS).await;
            select_block(device, subclass_id, block_index).await?;
            if read_block_checksum(device).await? != checksum {
                return Err(Error::BlockChecksumMismatch);
            }
        }

        Ok(())
    }

    pub async fn select_block<I, E>(
        device: &mut Bq27441Device<I>,
        subclass_id: u8,
        block_index: u8,
    ) -> Result<(), Error<E>>
    where
        I: AsyncRegisterInterface<AddressType = u8, Error = E>,
        E: Debug,
    {
        device
            .block_data_control()
            .write_async(|register| *register = field_sets::BlockDataControl::from([0u8]))
            .await
            .map_err(map_i2c)?;

        device
            .data_class()
            .write_async(|register| *register = field_sets::DataClass::from([subclass_id]))
            .await
            .map_err(map_i2c)?;

        device
            .data_block()
            .write_async(|register| *register = field_sets::DataBlock::from([block_index]))
            .await
            .map_err(map_i2c)?;

        Ok(())
    }

    async fn write_block_checksum<I, E>(
        device: &mut Bq27441Device<I>,
        checksum: u8,
    ) -> Result<(), Error<E>>
    where
        I: AsyncRegisterInterface<AddressType = u8, Error = E>,
        E: Debug,
    {
        device
            .block_data_checksum()
            .write_async(|register| *register = field_sets::BlockDataChecksum::from([checksum]))
            .await
            .map_err(map_i2c)
    }

    async fn read_block_raw<I, E>(
        device: &mut Bq27441Device<I>,
    ) -> Result<[u8; BLOCK_SIZE], Error<E>>
    where
        I: AsyncRegisterInterface<AddressType = u8, Error = E> + AsyncBufferInterface<AddressType = u8, Error = E>,
        E: Debug,
    {
        let mut block = [0u8; BLOCK_SIZE];
        device
            .block_data()
            .read_exact_async(&mut block)
            .await
            .map_err(map_read_exact)?;
        Ok(block)
    }

    pub async fn data_memory_read_block<I, E>(
        device: &mut Bq27441Device<I>,
        subclass_id: u8,
        block_index: u8,
    ) -> Result<[u8; BLOCK_SIZE], Error<E>>
    where
        I: AsyncRegisterInterface<AddressType = u8, Error = E> + AsyncBufferInterface<AddressType = u8, Error = E>,
        E: Debug,
    {
        select_block(device, subclass_id, block_index).await?;
        read_block_raw(device).await
    }

    pub async fn data_memory_write_block<I, E, D>(
        device: &mut Bq27441Device<I>,
        subclass_id: u8,
        block_index: u8,
        data: &[u8; BLOCK_SIZE],
        delay: &mut D,
    ) -> Result<(), Error<E>>
    where
        I: AsyncRegisterInterface<AddressType = u8, Error = E> + AsyncBufferInterface<AddressType = u8, Error = E>,
        E: Debug,
        D: DelayMsAsync,
    {
        data_memory_write_block_with_options(
            device,
            subclass_id,
            block_index,
            data,
            BlockWriteOptions::default(),
            delay,
        )
        .await
    }

    pub async fn data_memory_write_block_with_options<I, E, D>(
        device: &mut Bq27441Device<I>,
        subclass_id: u8,
        block_index: u8,
        data: &[u8; BLOCK_SIZE],
        options: BlockWriteOptions,
        delay: &mut D,
    ) -> Result<(), Error<E>>
    where
        I: AsyncRegisterInterface<AddressType = u8, Error = E> + AsyncBufferInterface<AddressType = u8, Error = E>,
        E: Debug,
        D: DelayMsAsync,
    {
        write_block_committed(device, subclass_id, block_index, data, options, delay).await
    }

    pub async fn data_memory_read_subclass<I, E>(
        device: &mut Bq27441Device<I>,
        subclass_id: u8,
        data: &mut [u8],
    ) -> Result<(), Error<E>>
    where
        I: AsyncRegisterInterface<AddressType = u8, Error = E> + AsyncBufferInterface<AddressType = u8, Error = E>,
        E: Debug,
    {
        validate_subclass_len(data.len()).map_err(|_| Error::InvalidParam)?;

        for (block_index, chunk) in data.chunks_mut(BLOCK_SIZE).enumerate() {
            let block = data_memory_read_block(device, subclass_id, block_index as u8).await?;
            chunk.copy_from_slice(&block);
        }

        Ok(())
    }

    pub async fn data_memory_write_subclass<I, E, D>(
        device: &mut Bq27441Device<I>,
        subclass_id: u8,
        data: &[u8],
        options: BlockWriteOptions,
        delay: &mut D,
    ) -> Result<(), Error<E>>
    where
        I: AsyncRegisterInterface<AddressType = u8, Error = E> + AsyncBufferInterface<AddressType = u8, Error = E>,
        E: Debug,
        D: DelayMsAsync,
    {
        validate_subclass_len(data.len()).map_err(|_| Error::InvalidParam)?;

        for (block_index, chunk) in data.chunks(BLOCK_SIZE).enumerate() {
            let mut block = [0u8; BLOCK_SIZE];
            block.copy_from_slice(chunk);
            write_block_committed(
                device,
                subclass_id,
                block_index as u8,
                &block,
                options,
                delay,
            )
            .await?;
        }

        Ok(())
    }

    pub async fn commit_block_checksum<I, E>(device: &mut Bq27441Device<I>) -> Result<(), Error<E>>
    where
        I: AsyncRegisterInterface<AddressType = u8, Error = E> + AsyncBufferInterface<AddressType = u8, Error = E>,
        E: Debug,
    {
        let block = read_block_raw(device).await?;
        write_block_checksum(device, block_checksum(&block)).await
    }

    pub async fn read_opconfig<I, E>(
        device: &mut Bq27441Device<I>,
    ) -> Result<field_sets::Opconfig, Error<E>>
    where
        I: AsyncRegisterInterface<AddressType = u8, Error = E>,
        E: Debug,
    {
        device.opconfig().read_async().await.map_err(map_i2c)
    }

    pub async fn read_design_capacity<I, E>(
        device: &mut Bq27441Device<I>,
    ) -> Result<u16, Error<E>>
    where
        I: AsyncRegisterInterface<AddressType = u8, Error = E>,
        E: Debug,
    {
        Ok(u16_le(
            device
                .design_capacity()
                .read_async()
                .await
                .map_err(map_i2c)?,
        ))
    }

    impl ConfigUpdateSession {
        /// Commit checksums for every marked block (re-selects each block first).
        pub async fn commit_all_async<I, E>(self, device: &mut Bq27441Device<I>) -> Result<(), Error<E>>
        where
            I: AsyncRegisterInterface<AddressType = u8, Error = E>
                + AsyncBufferInterface<AddressType = u8, Error = E>,
            E: Debug,
        {
            for block in self.pending.into_iter().flatten() {
                select_block(device, block.subclass_id, block.block_index).await?;
                commit_block_checksum(device).await?;
            }
            Ok(())
        }
    }
}

#[cfg(feature = "async")]
pub use async_ops::{
    data_memory_read_block as data_memory_read_block_async,
    data_memory_read_subclass as data_memory_read_subclass_async,
    data_memory_write_block as data_memory_write_block_async,
    data_memory_write_block_with_options as data_memory_write_block_with_options_async,
    data_memory_write_subclass as data_memory_write_subclass_async,
    read_design_capacity as read_design_capacity_async,
    read_opconfig as read_opconfig_async,
    select_block as select_block_async,
};

#[cfg(test)]
mod tests {
    use super::{block_checksum, patch_checksum, validate_subclass_len, ConfigUpdateSession, BLOCK_SIZE};
    use super::subclass;

    #[test]
    fn block_checksum_complements_sum() {
        let block = [0xFFu8; BLOCK_SIZE];
        let sum: u8 = block.iter().copied().fold(0u8, |acc, byte| acc.wrapping_add(byte));
        assert_eq!(block_checksum(&block), 255u8.wrapping_sub(sum));
    }

    #[test]
    fn patch_checksum_matches_trm_replacement() {
        let old = [0x03u8, 0xE8];
        let new = [0x04u8, 0xB0];
        let old_checksum = 0xE8;
        assert_eq!(patch_checksum(old_checksum, &old, &new), 0x1F);
    }

    #[test]
    fn config_update_session_deduplicates_blocks() {
        let mut session = ConfigUpdateSession::new();
        session.mark_block(subclass::STATE, 0);
        session.mark_block(subclass::STATE, 0);
        assert_eq!(session.pending_count(), 1);
    }

    #[test]
    fn validate_subclass_len_accepts_block_multiples() {
        assert!(validate_subclass_len(BLOCK_SIZE).is_ok());
        assert!(validate_subclass_len(BLOCK_SIZE * 2).is_ok());
        assert!(validate_subclass_len(0).is_err());
        assert!(validate_subclass_len(BLOCK_SIZE - 1).is_err());
    }
}
