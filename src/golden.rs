//! Golden-file data memory snapshot import/export (slua903 §3.2.7).

use core::fmt::Debug;

#[cfg(feature = "async")]
use device_driver::{AsyncBufferInterface, AsyncRegisterInterface};
use device_driver::{BufferInterface, RegisterInterface};

use crate::Error;
use crate::data_memory::{BLOCK_SIZE, data_memory_read_block, data_memory_write_block, subclass};
use crate::delay::DelayMs;
use crate::generated::Bq27441Device;

/// A single 32-byte data memory block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataMemoryBlock {
    /// Data memory subclass ID.
    pub subclass_id: u8,
    /// Block index within the subclass (0–31).
    pub block_index: u8,
    /// Raw block payload.
    pub data: [u8; BLOCK_SIZE],
}

/// Snapshot of the data memory blocks commonly exported as a golden file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoldenSnapshot {
    /// State subclass block 0 (Qmax, update status, design capacity, …).
    pub state_block0: [u8; BLOCK_SIZE],
    /// State subclass block 1 (charge termination voltage, delta voltage, …).
    pub state_block1: [u8; BLOCK_SIZE],
    /// Current thresholds subclass block 0.
    pub current_thresholds: [u8; BLOCK_SIZE],
    /// Registers subclass block 0 (`OpConfig`, `OpConfigB`).
    pub registers: [u8; BLOCK_SIZE],
    /// Ra table subclass block 0.
    pub ra_table: [u8; BLOCK_SIZE],
}

impl GoldenSnapshot {
    /// Read golden-file blocks from data memory (device must be unsealed).
    pub fn read<I, E>(device: &mut Bq27441Device<I>) -> Result<Self, Error<E>>
    where
        I: RegisterInterface<AddressType = u8, Error = E>
            + BufferInterface<AddressType = u8, Error = E>,
        E: Debug,
    {
        Ok(Self {
            state_block0: data_memory_read_block(device, subclass::STATE, 0)?,
            state_block1: data_memory_read_block(device, subclass::STATE, 1)?,
            current_thresholds: data_memory_read_block(device, subclass::CURRENT_THRESHOLDS, 0)?,
            registers: data_memory_read_block(device, subclass::REGISTERS, 0)?,
            ra_table: data_memory_read_block(device, subclass::RA_RAM, 0)?,
        })
    }

    /// Write golden-file blocks to data memory (device must be in CONFIG UPDATE mode).
    pub fn write<I, E, D>(
        self,
        device: &mut Bq27441Device<I>,
        delay: &mut D,
    ) -> Result<(), Error<E>>
    where
        I: RegisterInterface<AddressType = u8, Error = E>
            + BufferInterface<AddressType = u8, Error = E>,
        E: Debug,
        D: DelayMs,
    {
        data_memory_write_block(device, subclass::STATE, 0, &self.state_block0, delay)?;
        data_memory_write_block(device, subclass::STATE, 1, &self.state_block1, delay)?;
        data_memory_write_block(
            device,
            subclass::CURRENT_THRESHOLDS,
            0,
            &self.current_thresholds,
            delay,
        )?;
        data_memory_write_block(device, subclass::REGISTERS, 0, &self.registers, delay)?;
        data_memory_write_block(device, subclass::RA_RAM, 0, &self.ra_table, delay)?;
        Ok(())
    }

    /// Iterate over the contained blocks.
    #[must_use]
    pub fn blocks(&self) -> [DataMemoryBlock; 5] {
        [
            DataMemoryBlock {
                subclass_id: subclass::STATE,
                block_index: 0,
                data: self.state_block0,
            },
            DataMemoryBlock {
                subclass_id: subclass::STATE,
                block_index: 1,
                data: self.state_block1,
            },
            DataMemoryBlock {
                subclass_id: subclass::CURRENT_THRESHOLDS,
                block_index: 0,
                data: self.current_thresholds,
            },
            DataMemoryBlock {
                subclass_id: subclass::REGISTERS,
                block_index: 0,
                data: self.registers,
            },
            DataMemoryBlock {
                subclass_id: subclass::RA_RAM,
                block_index: 0,
                data: self.ra_table,
            },
        ]
    }
}

/// Read an arbitrary data memory block (device must be unsealed).
pub fn read_block<I, E>(
    device: &mut Bq27441Device<I>,
    subclass_id: u8,
    block_index: u8,
) -> Result<DataMemoryBlock, Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E>
        + BufferInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    Ok(DataMemoryBlock {
        subclass_id,
        block_index,
        data: data_memory_read_block(device, subclass_id, block_index)?,
    })
}

/// Write an arbitrary data memory block (device must be in CONFIG UPDATE mode).
pub fn write_block<I, E, D>(
    device: &mut Bq27441Device<I>,
    block: &DataMemoryBlock,
    delay: &mut D,
) -> Result<(), Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E>
        + BufferInterface<AddressType = u8, Error = E>,
    E: Debug,
    D: DelayMs,
{
    data_memory_write_block(
        device,
        block.subclass_id,
        block.block_index,
        &block.data,
        delay,
    )
}

#[cfg(feature = "async")]
mod async_ops {
    use super::{
        AsyncBufferInterface, AsyncRegisterInterface, Bq27441Device, DataMemoryBlock, Debug, Error,
        GoldenSnapshot, subclass,
    };
    use crate::delay::DelayMsAsync;

    impl GoldenSnapshot {
        /// Read golden-file blocks from data memory (device must be unsealed).
        pub async fn read_async<I, E>(device: &mut Bq27441Device<I>) -> Result<Self, Error<E>>
        where
            I: AsyncRegisterInterface<AddressType = u8, Error = E>
                + AsyncBufferInterface<AddressType = u8, Error = E>,
            E: Debug,
        {
            Ok(Self {
                state_block0: crate::data_memory::data_memory_read_block_async(
                    device,
                    subclass::STATE,
                    0,
                )
                .await?,
                state_block1: crate::data_memory::data_memory_read_block_async(
                    device,
                    subclass::STATE,
                    1,
                )
                .await?,
                current_thresholds: crate::data_memory::data_memory_read_block_async(
                    device,
                    subclass::CURRENT_THRESHOLDS,
                    0,
                )
                .await?,
                registers: crate::data_memory::data_memory_read_block_async(
                    device,
                    subclass::REGISTERS,
                    0,
                )
                .await?,
                ra_table: crate::data_memory::data_memory_read_block_async(
                    device,
                    subclass::RA_RAM,
                    0,
                )
                .await?,
            })
        }

        /// Write golden-file blocks to data memory (device must be in CONFIG UPDATE mode).
        pub async fn write_async<I, E, D>(
            self,
            device: &mut Bq27441Device<I>,
            delay: &mut D,
        ) -> Result<(), Error<E>>
        where
            I: AsyncRegisterInterface<AddressType = u8, Error = E>
                + AsyncBufferInterface<AddressType = u8, Error = E>,
            E: Debug,
            D: DelayMsAsync,
        {
            crate::data_memory::data_memory_write_block_async(
                device,
                subclass::STATE,
                0,
                &self.state_block0,
                delay,
            )
            .await?;
            crate::data_memory::data_memory_write_block_async(
                device,
                subclass::STATE,
                1,
                &self.state_block1,
                delay,
            )
            .await?;
            crate::data_memory::data_memory_write_block_async(
                device,
                subclass::CURRENT_THRESHOLDS,
                0,
                &self.current_thresholds,
                delay,
            )
            .await?;
            crate::data_memory::data_memory_write_block_async(
                device,
                subclass::REGISTERS,
                0,
                &self.registers,
                delay,
            )
            .await?;
            crate::data_memory::data_memory_write_block_async(
                device,
                subclass::RA_RAM,
                0,
                &self.ra_table,
                delay,
            )
            .await?;
            Ok(())
        }
    }

    pub async fn read_block<I, E>(
        device: &mut Bq27441Device<I>,
        subclass_id: u8,
        block_index: u8,
    ) -> Result<DataMemoryBlock, Error<E>>
    where
        I: AsyncRegisterInterface<AddressType = u8, Error = E>
            + AsyncBufferInterface<AddressType = u8, Error = E>,
        E: Debug,
    {
        Ok(DataMemoryBlock {
            subclass_id,
            block_index,
            data: crate::data_memory::data_memory_read_block_async(
                device,
                subclass_id,
                block_index,
            )
            .await?,
        })
    }
}

#[cfg(feature = "async")]
pub use async_ops::read_block as read_block_async;
