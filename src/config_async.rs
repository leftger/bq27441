//! Async battery configuration helpers using generated data-memory layouts.

use core::fmt::Debug;

use device_driver::{AsyncBufferInterface, AsyncRegisterInterface};

use crate::config::{
    BatteryConfig, ChargeTerminationThresholds, CurrentThresholds, DischargeThresholds, OpConfig,
    OpConfigB, RaTable, SafetyThresholds, UpdateStatus,
};
use crate::data_memory::{
    i16_field, i16_le, select_block_async, subclass, u16_field, u16_le, u8_le, ConfigUpdateSession,
    RA_TABLE_LEN, read_design_capacity_async, read_opconfig_async,
};
use crate::generated::Bq27441Device;
use crate::{field_sets, Error};

async fn read_ra_table<I, E>(device: &mut Bq27441Device<I>) -> Result<RaTable, Error<E>>
where
    I: AsyncRegisterInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    select_block_async(device, subclass::RA_RAM, 0).await?;
    let mut entries = [0u16; RA_TABLE_LEN];
    for (index, entry) in entries.iter_mut().enumerate() {
        *entry = u16_le(
            device
                .dm_ra_ram_block_0()
                .dm_ra_entry(index)
                .read_async()
                .await
                .map_err(Error::I2c)?,
        );
    }
    Ok(RaTable { entries })
}

pub async fn read_battery_config<I, E>(
    device: &mut Bq27441Device<I>,
) -> Result<BatteryConfig, Error<E>>
where
    I: AsyncRegisterInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    select_block_async(device, subclass::STATE, 0).await?;
    let design_capacity_mah = u16_le(
        device
            .dm_state_block_0()
            .dm_design_capacity()
            .read_async()
            .await
            .map_err(Error::I2c)?,
    );
    let design_energy_mwh = u16_le(
        device
            .dm_state_block_0()
            .dm_design_energy()
            .read_async()
            .await
            .map_err(Error::I2c)?,
    );
    let terminate_voltage_mv = u16_le(
        device
            .dm_state_block_0()
            .dm_terminate_voltage()
            .read_async()
            .await
            .map_err(Error::I2c)?,
    );
    let taper_voltage_mv = u16_le(
        device
            .dm_state_block_0()
            .dm_taper_voltage()
            .read_async()
            .await
            .map_err(Error::I2c)?,
    );
    let sleep_current_ma = u16::from(u8_le(
        device
            .dm_state_block_0()
            .dm_sleep_current()
            .read_async()
            .await
            .map_err(Error::I2c)?,
    ));
    let taper_rate = i16_le(
        device
            .dm_state_block_0()
            .dm_taper_rate()
            .read_async()
            .await
            .map_err(Error::I2c)?,
    );
    let reserve_cap_mah = u16_le(
        device
            .dm_state_block_0()
            .dm_reserve_cap()
            .read_async()
            .await
            .map_err(Error::I2c)?,
    );
    let update_status = UpdateStatus(u8_le(
        device
            .dm_state_block_0()
            .dm_update_status()
            .read_async()
            .await
            .map_err(Error::I2c)?,
    ));

    select_block_async(device, subclass::STATE, 1).await?;
    let charge_termination_voltage_mv = u16_le(
        device
            .dm_state_block_1()
            .dm_v_at_chg_term()
            .read_async()
            .await
            .map_err(Error::I2c)?,
    );

    select_block_async(device, subclass::REGISTERS, 0).await?;
    let op_config = OpConfig(u16_le(
        device
            .dm_registers_block_0()
            .dm_op_config()
            .read_async()
            .await
            .map_err(Error::I2c)?,
    ));
    let op_config_b = OpConfigB(u8_le(
        device
            .dm_registers_block_0()
            .dm_op_config_b()
            .read_async()
            .await
            .map_err(Error::I2c)?,
    ));

    select_block_async(device, subclass::CURRENT_THRESHOLDS, 0).await?;
    let current_thresholds = CurrentThresholds {
        dsg_rate: i16_le(
            device
                .dm_current_thresholds_block_0()
                .dm_dsg_current()
                .read_async()
                .await
                .map_err(Error::I2c)?,
        ),
        chg_rate: i16_le(
            device
                .dm_current_thresholds_block_0()
                .dm_chg_current()
                .read_async()
                .await
                .map_err(Error::I2c)?,
        ),
        quit_rate: i16_le(
            device
                .dm_current_thresholds_block_0()
                .dm_quit_current()
                .read_async()
                .await
                .map_err(Error::I2c)?,
        ),
    };

    select_block_async(device, subclass::SAFETY, 0).await?;
    let safety = SafetyThresholds {
        over_temp_deci_c: i16_le(
            device
                .dm_safety_block_0()
                .dm_over_temp()
                .read_async()
                .await
                .map_err(Error::I2c)?,
        ),
        under_temp_deci_c: i16_le(
            device
                .dm_safety_block_0()
                .dm_under_temp()
                .read_async()
                .await
                .map_err(Error::I2c)?,
        ),
        temp_hys_deci_c: u8_le(
            device
                .dm_safety_block_0()
                .dm_temp_hys()
                .read_async()
                .await
                .map_err(Error::I2c)?,
        ),
    };

    select_block_async(device, subclass::DISCHARGE, 0).await?;
    let discharge = DischargeThresholds {
        soc1_set_pct: u8_le(
            device
                .dm_discharge_block_0()
                .dm_soc_1_set()
                .read_async()
                .await
                .map_err(Error::I2c)?,
        ),
        soc1_clear_pct: u8_le(
            device
                .dm_discharge_block_0()
                .dm_soc_1_clear()
                .read_async()
                .await
                .map_err(Error::I2c)?,
        ),
        socf_set_pct: u8_le(
            device
                .dm_discharge_block_0()
                .dm_socf_set()
                .read_async()
                .await
                .map_err(Error::I2c)?,
        ),
        socf_clear_pct: u8_le(
            device
                .dm_discharge_block_0()
                .dm_socf_clear()
                .read_async()
                .await
                .map_err(Error::I2c)?,
        ),
    };

    select_block_async(device, subclass::CHARGE_TERMINATION, 0).await?;
    let charge_termination = ChargeTerminationThresholds {
        tca_set_pct: u8_le(
            device
                .dm_charge_termination_block_0()
                .dm_tca_set()
                .read_async()
                .await
                .map_err(Error::I2c)?,
        ) as i8,
        tca_clear_pct: u8_le(
            device
                .dm_charge_termination_block_0()
                .dm_tca_clear()
                .read_async()
                .await
                .map_err(Error::I2c)?,
        ) as i8,
        fc_set_pct: u8_le(
            device
                .dm_charge_termination_block_0()
                .dm_fc_set()
                .read_async()
                .await
                .map_err(Error::I2c)?,
        ) as i8,
        fc_clear_pct: u8_le(
            device
                .dm_charge_termination_block_0()
                .dm_fc_clear()
                .read_async()
                .await
                .map_err(Error::I2c)?,
        ) as i8,
    };

    Ok(BatteryConfig {
        design_capacity_mah,
        design_energy_mwh,
        terminate_voltage_mv,
        taper_voltage_mv,
        charge_termination_voltage_mv,
        sleep_current_ma,
        taper_rate,
        reserve_cap_mah,
        op_config,
        op_config_b,
        current_thresholds: Some(current_thresholds),
        update_status: Some(update_status),
        safety: Some(safety),
        discharge: Some(discharge),
        charge_termination: Some(charge_termination),
        ra_table: Some(read_ra_table(device).await?),
    })
}

pub async fn apply_battery_config<I, E>(
    device: &mut Bq27441Device<I>,
    config: &BatteryConfig,
) -> Result<(), Error<E>>
where
    I: AsyncRegisterInterface<AddressType = u8, Error = E> + AsyncBufferInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    config.validate().map_err(|_| Error::InvalidParam)?;

    let mut session = ConfigUpdateSession::new();

    select_block_async(device, subclass::STATE, 0).await?;
    device
        .dm_state_block_0()
        .dm_design_capacity()
        .write_async(|register| {
            *register =
                field_sets::DmDesignCapacity::from(i16_field(config.design_capacity_mah as i16));
        })
        .await
        .map_err(Error::I2c)?;
    device
        .dm_state_block_0()
        .dm_design_energy()
        .write_async(|register| {
            *register = field_sets::DmDesignEnergy::from(i16_field(config.design_energy_mwh as i16));
        })
        .await
        .map_err(Error::I2c)?;
    device
        .dm_state_block_0()
        .dm_terminate_voltage()
        .write_async(|register| {
            *register = field_sets::DmTerminateVoltage::from(i16_field(
                config.terminate_voltage_mv as i16,
            ));
        })
        .await
        .map_err(Error::I2c)?;
    if let Some(status) = config.update_status {
        device
            .dm_state_block_0()
            .dm_update_status()
            .write_async(|register| *register = field_sets::DmUpdateStatus::from([status.0]))
            .await
            .map_err(Error::I2c)?;
    }
    device
        .dm_state_block_0()
        .dm_reserve_cap()
        .write_async(|register| {
            *register = field_sets::DmReserveCap::from(i16_field(config.reserve_cap_mah as i16));
        })
        .await
        .map_err(Error::I2c)?;
    device
        .dm_state_block_0()
        .dm_taper_rate()
        .write_async(|register| {
            *register = field_sets::DmTaperRate::from(i16_field(config.taper_rate));
        })
        .await
        .map_err(Error::I2c)?;
    device
        .dm_state_block_0()
        .dm_taper_voltage()
        .write_async(|register| {
            *register =
                field_sets::DmTaperVoltage::from(i16_field(config.taper_voltage_mv as i16));
        })
        .await
        .map_err(Error::I2c)?;
    device
        .dm_state_block_0()
        .dm_sleep_current()
        .write_async(|register| {
            *register = field_sets::DmSleepCurrent::from([config.sleep_current_ma as u8]);
        })
        .await
        .map_err(Error::I2c)?;
    session.mark_block(subclass::STATE, 0);

    select_block_async(device, subclass::STATE, 1).await?;
    device
        .dm_state_block_1()
        .dm_v_at_chg_term()
        .write_async(|register| {
            *register = field_sets::DmVAtChgTerm::from(i16_field(
                config.charge_termination_voltage_mv as i16,
            ));
        })
        .await
        .map_err(Error::I2c)?;
    session.mark_block(subclass::STATE, 1);

    select_block_async(device, subclass::REGISTERS, 0).await?;
    device
        .dm_registers_block_0()
        .dm_op_config()
        .write_async(|register| {
            *register = field_sets::DmOpConfig::from(u16_field(config.op_config.0));
        })
        .await
        .map_err(Error::I2c)?;
    device
        .dm_registers_block_0()
        .dm_op_config_b()
        .write_async(|register| *register = field_sets::DmOpConfigB::from([config.op_config_b.0]))
        .await
        .map_err(Error::I2c)?;
    session.mark_block(subclass::REGISTERS, 0);

    if let Some(safety) = config.safety {
        safety.validate().map_err(|_| Error::InvalidParam)?;
        select_block_async(device, subclass::SAFETY, 0).await?;
        device
            .dm_safety_block_0()
            .dm_over_temp()
            .write_async(|register| {
                *register = field_sets::DmOverTemp::from(i16_field(safety.over_temp_deci_c));
            })
            .await
            .map_err(Error::I2c)?;
        device
            .dm_safety_block_0()
            .dm_under_temp()
            .write_async(|register| {
                *register = field_sets::DmUnderTemp::from(i16_field(safety.under_temp_deci_c));
            })
            .await
            .map_err(Error::I2c)?;
        device
            .dm_safety_block_0()
            .dm_temp_hys()
            .write_async(|register| *register = field_sets::DmTempHys::from([safety.temp_hys_deci_c]))
            .await
            .map_err(Error::I2c)?;
        session.mark_block(subclass::SAFETY, 0);
    }

    if let Some(discharge) = config.discharge {
        discharge.validate().map_err(|_| Error::InvalidParam)?;
        select_block_async(device, subclass::DISCHARGE, 0).await?;
        device
            .dm_discharge_block_0()
            .dm_soc_1_set()
            .write_async(|register| *register = field_sets::DmSoc1Set::from([discharge.soc1_set_pct]))
            .await
            .map_err(Error::I2c)?;
        device
            .dm_discharge_block_0()
            .dm_soc_1_clear()
            .write_async(|register| {
                *register = field_sets::DmSoc1Clear::from([discharge.soc1_clear_pct]);
            })
            .await
            .map_err(Error::I2c)?;
        device
            .dm_discharge_block_0()
            .dm_socf_set()
            .write_async(|register| *register = field_sets::DmSocfSet::from([discharge.socf_set_pct]))
            .await
            .map_err(Error::I2c)?;
        device
            .dm_discharge_block_0()
            .dm_socf_clear()
            .write_async(|register| {
                *register = field_sets::DmSocfClear::from([discharge.socf_clear_pct]);
            })
            .await
            .map_err(Error::I2c)?;
        session.mark_block(subclass::DISCHARGE, 0);
    }

    if let Some(charge) = config.charge_termination {
        charge.validate().map_err(|_| Error::InvalidParam)?;
        select_block_async(device, subclass::CHARGE_TERMINATION, 0).await?;
        device
            .dm_charge_termination_block_0()
            .dm_tca_set()
            .write_async(|register| *register = field_sets::DmTcaSet::from([charge.tca_set_pct as u8]))
            .await
            .map_err(Error::I2c)?;
        device
            .dm_charge_termination_block_0()
            .dm_tca_clear()
            .write_async(|register| {
                *register = field_sets::DmTcaClear::from([charge.tca_clear_pct as u8]);
            })
            .await
            .map_err(Error::I2c)?;
        device
            .dm_charge_termination_block_0()
            .dm_fc_set()
            .write_async(|register| *register = field_sets::DmFcSet::from([charge.fc_set_pct as u8]))
            .await
            .map_err(Error::I2c)?;
        device
            .dm_charge_termination_block_0()
            .dm_fc_clear()
            .write_async(|register| {
                *register = field_sets::DmFcClear::from([charge.fc_clear_pct as u8]);
            })
            .await
            .map_err(Error::I2c)?;
        session.mark_block(subclass::CHARGE_TERMINATION, 0);
    }

    if let Some(thresholds) = config.current_thresholds {
        thresholds
            .validate_with_taper(config.taper_rate)
            .map_err(|_| Error::InvalidParam)?;
        select_block_async(device, subclass::CURRENT_THRESHOLDS, 0).await?;
        device
            .dm_current_thresholds_block_0()
            .dm_dsg_current()
            .write_async(|register| {
                *register = field_sets::DmDsgCurrent::from(i16_field(thresholds.dsg_rate));
            })
            .await
            .map_err(Error::I2c)?;
        device
            .dm_current_thresholds_block_0()
            .dm_chg_current()
            .write_async(|register| {
                *register = field_sets::DmChgCurrent::from(i16_field(thresholds.chg_rate));
            })
            .await
            .map_err(Error::I2c)?;
        device
            .dm_current_thresholds_block_0()
            .dm_quit_current()
            .write_async(|register| {
                *register = field_sets::DmQuitCurrent::from(i16_field(thresholds.quit_rate));
            })
            .await
            .map_err(Error::I2c)?;
        session.mark_block(subclass::CURRENT_THRESHOLDS, 0);
    }

    if let Some(ra) = config.ra_table {
        ra.validate().map_err(|_| Error::InvalidParam)?;
        select_block_async(device, subclass::RA_RAM, 0).await?;
        for (index, &entry) in ra.entries.iter().enumerate() {
            device
                .dm_ra_ram_block_0()
                .dm_ra_entry(index)
                .write_async(|register| {
                    *register = field_sets::DmRaEntry::from(i16_field(entry as i16));
                })
                .await
                .map_err(Error::I2c)?;
        }
        session.mark_block(subclass::RA_RAM, 0);
    }

    session.commit_all_async(device).await
}

pub async fn design_capacity<I, E>(device: &mut Bq27441Device<I>) -> Result<u16, Error<E>>
where
    I: AsyncRegisterInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    read_design_capacity_async(device).await
}

pub async fn op_config<I, E>(device: &mut Bq27441Device<I>) -> Result<OpConfig, Error<E>>
where
    I: AsyncRegisterInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    read_opconfig_async(device)
        .await
        .map(|register| OpConfig(u16_le(register)))
}
