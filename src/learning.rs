//! Learning-cycle monitoring helpers (slua903, TRM §4.1.1, §6.4.2.3.2).

use core::fmt::Debug;

#[cfg(feature = "async")]
use device_driver::AsyncRegisterInterface;
use device_driver::RegisterInterface;

use crate::Error;
use crate::config::{ControlStatus, LearningPhase, UpdateStatus};
use crate::data_memory::{i16_le, select_block, subclass, u8_le};
use crate::generated::Bq27441Device;

/// Snapshot of learning-cycle progress from control status and flags.
///
/// Each field mirrors an independent hardware status bit rather than a
/// mutually-exclusive state, so this isn't a good fit for an enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct LearningProgress {
    /// Raw Update Status byte from data memory (`None` when device is sealed).
    pub update_status: Option<u8>,
    /// `CONTROL_STATUS[QMAX_UP]`.
    pub qmax_updated: bool,
    /// `CONTROL_STATUS[RES_UP]`.
    pub resistance_updated: bool,
    /// `CONTROL_STATUS[VOK]`.
    pub voltages_ok_for_qmax: bool,
    /// `CONTROL_STATUS[RUP_DIS]`.
    pub ra_updates_disabled: bool,
    /// `Flags()[FC]`.
    pub full_charge_detected: bool,
    /// `Flags()[OCVTAKEN]`.
    pub ocv_taken: bool,
}

impl LearningProgress {
    /// Decode the system-side learning phase when Update Status is available.
    #[must_use]
    pub const fn phase(&self) -> Option<LearningPhase> {
        match self.update_status {
            Some(value) => Some(LearningPhase::from_update_status(value)),
            None => None,
        }
    }
}

/// Read Update Status from data memory (device must be unsealed).
pub fn read_update_status<I, E>(device: &mut Bq27441Device<I>) -> Result<UpdateStatus, Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    select_block(device, subclass::STATE, 0)?;
    Ok(UpdateStatus(u8_le(
        device
            .dm_state_block_0()
            .dm_update_status()
            .read()
            .map_err(Error::I2c)?,
    )))
}

/// Read normalized Qmax Cell 0 from data memory (device must be unsealed).
pub fn read_qmax_cell_0<I, E>(device: &mut Bq27441Device<I>) -> Result<i16, Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    select_block(device, subclass::STATE, 0)?;
    Ok(i16_le(
        device
            .dm_state_block_0()
            .dm_qmax_cell_0()
            .read()
            .map_err(Error::I2c)?,
    ))
}

/// Convert Qmax Cell 0 and design capacity to learned Qmax in mAh (TRM §6.4.2.3.1).
#[must_use]
pub fn qmax_mah(qmax_cell_0: i16, design_capacity_mah: u16) -> u32 {
    let qmax = i32::from(qmax_cell_0) * i32::from(design_capacity_mah);
    (qmax / 16_384).cast_unsigned()
}

/// Read learned Delta Voltage in mV (device must be unsealed).
pub fn read_delta_voltage<I, E>(device: &mut Bq27441Device<I>) -> Result<i16, Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    select_block(device, subclass::STATE, 1)?;
    Ok(i16_le(
        device
            .dm_state_block_1()
            .dm_delta_voltage()
            .read()
            .map_err(Error::I2c)?,
    ))
}

/// Poll control status and flags; read Update Status when the device is unsealed.
pub fn learning_progress<I, E, F>(
    device: &mut Bq27441Device<I>,
    status: ControlStatus,
    mut flags_read: F,
    sealed: bool,
) -> Result<LearningProgress, Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E>,
    E: Debug,
    F: FnMut(&mut Bq27441Device<I>) -> Result<crate::field_sets::Flags, Error<E>>,
{
    let flags = flags_read(device)?;

    let update_status = if sealed {
        None
    } else {
        Some(read_update_status(device)?.0)
    };

    Ok(LearningProgress {
        update_status,
        qmax_updated: status.qmax_updated(),
        resistance_updated: status.resistance_updated(),
        voltages_ok_for_qmax: status.voltages_ok_for_qmax(),
        ra_updates_disabled: status.ra_updates_disabled(),
        full_charge_detected: flags.fc(),
        ocv_taken: flags.ocvtaken(),
    })
}

#[cfg(feature = "async")]
mod async_ops {
    use super::{
        AsyncRegisterInterface, Bq27441Device, ControlStatus, Debug, Error, LearningProgress,
        UpdateStatus, i16_le, subclass, u8_le,
    };
    use crate::data_memory::select_block_async;

    pub async fn read_update_status<I, E>(
        device: &mut Bq27441Device<I>,
    ) -> Result<UpdateStatus, Error<E>>
    where
        I: AsyncRegisterInterface<AddressType = u8, Error = E>,
        E: Debug,
    {
        select_block_async(device, subclass::STATE, 0).await?;
        Ok(UpdateStatus(u8_le(
            device
                .dm_state_block_0()
                .dm_update_status()
                .read_async()
                .await
                .map_err(Error::I2c)?,
        )))
    }

    pub async fn read_qmax_cell_0<I, E>(device: &mut Bq27441Device<I>) -> Result<i16, Error<E>>
    where
        I: AsyncRegisterInterface<AddressType = u8, Error = E>,
        E: Debug,
    {
        select_block_async(device, subclass::STATE, 0).await?;
        Ok(i16_le(
            device
                .dm_state_block_0()
                .dm_qmax_cell_0()
                .read_async()
                .await
                .map_err(Error::I2c)?,
        ))
    }

    pub async fn read_delta_voltage<I, E>(device: &mut Bq27441Device<I>) -> Result<i16, Error<E>>
    where
        I: AsyncRegisterInterface<AddressType = u8, Error = E>,
        E: Debug,
    {
        select_block_async(device, subclass::STATE, 1).await?;
        Ok(i16_le(
            device
                .dm_state_block_1()
                .dm_delta_voltage()
                .read_async()
                .await
                .map_err(Error::I2c)?,
        ))
    }

    pub async fn learning_progress<I, E>(
        device: &mut Bq27441Device<I>,
        status: ControlStatus,
        flags: crate::field_sets::Flags,
        sealed: bool,
    ) -> Result<LearningProgress, Error<E>>
    where
        I: AsyncRegisterInterface<AddressType = u8, Error = E>,
        E: Debug,
    {
        let update_status = if sealed {
            None
        } else {
            Some(read_update_status(device).await?.0)
        };

        Ok(LearningProgress {
            update_status,
            qmax_updated: status.qmax_updated(),
            resistance_updated: status.resistance_updated(),
            voltages_ok_for_qmax: status.voltages_ok_for_qmax(),
            ra_updates_disabled: status.ra_updates_disabled(),
            full_charge_detected: flags.fc(),
            ocv_taken: flags.ocvtaken(),
        })
    }
}

#[cfg(feature = "async")]
pub use async_ops::{
    learning_progress as learning_progress_async, read_delta_voltage as read_delta_voltage_async,
    read_qmax_cell_0 as read_qmax_cell_0_async, read_update_status as read_update_status_async,
};
