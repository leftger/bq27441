//! Battery configuration via Data Memory (TRM §3.1, §6.4).

use core::fmt::Debug;

use device_driver::{BufferInterface, RegisterInterface};

use crate::data_memory::{
    i16_field, i16_le, select_block, subclass, u16_field, u16_le, u8_le, ConfigUpdateSession,
    RA_TABLE_LEN, read_design_capacity, read_opconfig,
};
use crate::delay::{BusyWait, DelayMs};
#[cfg(feature = "async")]
use crate::delay::DelayMsAsync;
use crate::{field_sets, Bq27441Device, ChemId, ControlCmd, Error};

/// Default poll interval while waiting for CONFIG UPDATE mode transitions.
const CONFIG_POLL_INTERVAL_MS: u32 = 10;

/// Maximum time to wait for CONFIG UPDATE mode transitions.
const CONFIG_TIMEOUT_MS: u32 = 1_000;

/// How to leave CONFIG UPDATE mode after applying configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigExit {
    /// Soft reset — exits config and takes an OCV measurement (`Control 0x0042`).
    SoftReset,
    /// Resimulate with updated configuration and refresh SOC (`Control 0x0044`).
    ExitResim,
    /// Exit without OCV measurement or resimulation (`Control 0x0043`).
    ExitNoResim,
}

impl Default for ConfigExit {
    fn default() -> Self {
        Self::ExitResim
    }
}

/// Options for [`Bq27441::configure_battery`] / [`Bq27441Async::configure_battery`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigureOptions {
    /// Unseal the device when it is sealed (default: `true`).
    pub unseal: bool,
    /// Return to sealed mode after applying configuration (default: `true`).
    pub reseal: bool,
    /// How to exit CONFIG UPDATE mode (default: [`ConfigExit::ExitResim`]).
    pub exit: ConfigExit,
}

impl Default for ConfigureOptions {
    fn default() -> Self {
        Self {
            unseal: true,
            reseal: true,
            exit: ConfigExit::ExitResim,
        }
    }
}

/// Parsed `CONTROL_STATUS` subcommand response (TRM §4.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlStatus {
    /// Raw 16-bit status word.
    pub raw: u16,
}

impl ControlStatus {
    /// Device is in SEALED access mode (`SS`).
    #[must_use]
    pub const fn sealed(self) -> bool {
        (self.raw & 0x2000) != 0
    }

    /// Device is in CONFIG UPDATE calibration mode (`CALMODE`).
    #[must_use]
    pub const fn cal_mode(self) -> bool {
        (self.raw & 0x1000) != 0
    }

    /// Shutdown mode is enabled (`SHUTDOWNEN`).
    #[must_use]
    pub const fn shutdown_enabled(self) -> bool {
        (self.raw & 0x8000) != 0
    }

    /// Constant-power load model is selected (`LDMD`).
    #[must_use]
    pub const fn load_mode_power(self) -> bool {
        (self.raw & 0x0008) != 0
    }

    /// Watchdog reset occurred (`WDRESET`).
    #[must_use]
    pub const fn watchdog_reset(self) -> bool {
        (self.raw & 0x4000) != 0
    }

    /// Coulomb counter auto-calibration is active (`CCA`).
    #[must_use]
    pub const fn cc_auto_calibration(self) -> bool {
        (self.raw & 0x0800) != 0
    }

    /// Board calibration routine is active (`BCA`).
    #[must_use]
    pub const fn board_calibration(self) -> bool {
        (self.raw & 0x0400) != 0
    }

    /// Qmax has been updated (`QMAX_UP`).
    #[must_use]
    pub const fn qmax_updated(self) -> bool {
        (self.raw & 0x0200) != 0
    }

    /// Resistance table has been updated (`RES_UP`).
    #[must_use]
    pub const fn resistance_updated(self) -> bool {
        (self.raw & 0x0100) != 0
    }

    /// Initialization is complete (`INITCOMP`).
    #[must_use]
    pub const fn init_complete(self) -> bool {
        (self.raw & 0x0080) != 0
    }

    /// Hibernate entry has been requested (`HIBERNATE`).
    #[must_use]
    pub const fn hibernate(self) -> bool {
        (self.raw & 0x0040) != 0
    }

    /// Device is in sleep mode (`SLEEP`).
    #[must_use]
    pub const fn sleep(self) -> bool {
        (self.raw & 0x0010) != 0
    }

    /// Ra table updates are disabled (`RUP_DIS`).
    #[must_use]
    pub const fn ra_updates_disabled(self) -> bool {
        (self.raw & 0x0004) != 0
    }

    /// Cell voltages are qualified for Qmax update (`VOK`).
    #[must_use]
    pub const fn voltages_ok_for_qmax(self) -> bool {
        (self.raw & 0x0002) != 0
    }
}

/// System-side learning-cycle phase encoded in Update Status (slua903 §3 footnote).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LearningPhase {
    /// Learning not started (`0x00`).
    NotStarted,
    /// First Qmax update completed (`0x01`).
    FirstQmaxDone,
    /// Learning cycle complete (`0x02`).
    Complete,
    /// Unrecognized Update Status value.
    Unknown(u8),
}

impl LearningPhase {
    /// Decode the BQ27441 system-side Update Status low nibble.
    #[must_use]
    pub const fn from_update_status(value: u8) -> Self {
        match value & 0x0F {
            0x00 => Self::NotStarted,
            0x01 => Self::FirstQmaxDone,
            0x02 => Self::Complete,
            other => Self::Unknown(other),
        }
    }
}

/// Update Status data-flash byte (TRM §6.4.2.3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateStatus(pub u8);

impl UpdateStatus {
    /// Factory default (`0x00`).
    pub const DEFAULT: Self = Self(0);

    /// Enable unlimited Qmax/Ra updates during initial learning (bits 0 and 1 set).
    pub const INITIAL_LEARNING: Self = Self(0x03);

    /// System-side learning complete (slua903 §3 footnote).
    pub const LEARNING_COMPLETE: Self = Self(0x02);

    /// Default to SEALED on POR / config exit when bit 7 is set.
    #[must_use]
    pub const fn default_sealed(self) -> bool {
        (self.0 & 0x80) != 0
    }

    /// Qmax and Ra update limits are disabled (bits 0 and 1 both set).
    #[must_use]
    pub const fn qmax_ra_limits_disabled(self) -> bool {
        (self.0 & 0x03) == 0x03
    }

    /// Return the system-side learning phase.
    #[must_use]
    pub const fn phase(self) -> LearningPhase {
        LearningPhase::from_update_status(self.0)
    }

    /// Set or clear the default-sealed bit (bit 7).
    #[must_use]
    pub const fn with_default_sealed(self, enabled: bool) -> Self {
        if enabled {
            Self(self.0 | 0x80)
        } else {
            Self(self.0 & !0x80)
        }
    }

    /// Enable unlimited Qmax/Ra updates for initial learning.
    #[must_use]
    pub const fn for_initial_learning(self) -> Self {
        Self((self.0 & !0x03) | 0x03)
    }
}

/// Current detection thresholds (Current Thresholds subclass, TRM §6.4.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CurrentThresholds {
    /// Discharge current threshold (0.1 Hr rate, default 167 ≈ C/16.7).
    pub dsg_rate: i16,
    /// Charge current threshold (0.1 Hr rate, default 100 ≈ C/10).
    pub chg_rate: i16,
    /// Quit / relax current threshold (0.1 Hr rate, default 250 ≈ C/25).
    pub quit_rate: i16,
}

impl CurrentThresholds {
    /// TRM default values.
    pub const DEFAULT: Self = Self {
        dsg_rate: 167,
        chg_rate: 100,
        quit_rate: 250,
    };

    /// Effective discharge threshold in mA for a given design capacity.
    #[must_use]
    pub fn dsg_current_ma(self, design_capacity_mah: u16) -> u32 {
        Self::rate_to_current_ma(design_capacity_mah, self.dsg_rate)
    }

    /// Effective charge threshold in mA for a given design capacity.
    #[must_use]
    pub fn chg_current_ma(self, design_capacity_mah: u16) -> u32 {
        Self::rate_to_current_ma(design_capacity_mah, self.chg_rate)
    }

    /// Effective quit threshold in mA for a given design capacity.
    #[must_use]
    pub fn quit_current_ma(self, design_capacity_mah: u16) -> u32 {
        Self::rate_to_current_ma(design_capacity_mah, self.quit_rate)
    }

    /// Build thresholds from desired currents in mA.
    #[must_use]
    pub fn from_currents_ma(
        design_capacity_mah: u16,
        dsg_ma: u16,
        chg_ma: u16,
        quit_ma: u16,
    ) -> Self {
        Self {
            dsg_rate: Self::current_ma_to_rate(design_capacity_mah, dsg_ma),
            chg_rate: Self::current_ma_to_rate(design_capacity_mah, chg_ma),
            quit_rate: Self::current_ma_to_rate(design_capacity_mah, quit_ma),
        }
    }

    fn rate_to_current_ma(design_capacity_mah: u16, rate: i16) -> u32 {
        if rate <= 0 {
            return 0;
        }
        u32::from(design_capacity_mah) * 10 / u32::try_from(rate).unwrap_or(1)
    }

    fn current_ma_to_rate(design_capacity_mah: u16, current_ma: u16) -> i16 {
        if current_ma == 0 {
            return 0;
        }
        (u32::from(design_capacity_mah) * 10 / u32::from(current_ma)) as i16
    }

    fn validate(self) -> Result<Self, Error<()>> {
        range_i16(self.dsg_rate, 0, 2000, "dsg_rate")?;
        range_i16(self.chg_rate, 0, 2000, "chg_rate")?;
        range_i16(self.quit_rate, 0, 2000, "quit_rate")?;
        if self.dsg_rate > self.chg_rate || self.chg_rate >= self.quit_rate {
            return Err(Error::InvalidParam);
        }
        Ok(self)
    }

    /// Return `true` when dsg/chg/quit ordering satisfies slua903.
    #[must_use]
    pub const fn ordering_valid(self) -> bool {
        self.dsg_rate <= self.chg_rate && self.chg_rate < self.quit_rate
    }
}

impl CurrentThresholds {
    /// Validate ordering against a taper rate (TRM §6.4.2.3.11, slua903 §2).
    pub fn validate_with_taper(self, taper_rate: i16) -> Result<Self, Error<()>> {
        range_i16(taper_rate, 0, 2000, "taper_rate")?;
        self.validate()?;
        if taper_rate > self.chg_rate {
            return Err(Error::InvalidParam);
        }
        Ok(self)
    }
}

/// Operation configuration register (`OpConfig`, TRM Table 6-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpConfig(pub u16);

impl OpConfig {
    /// Default for BQ27441-G1A/G1B (`0x25F8`).
    pub const DEFAULT: Self = Self(0x25F8);

    fn register(self) -> field_sets::Opconfig {
        field_sets::Opconfig::from(self.0.to_le_bytes())
    }

    fn with_register(self, register: field_sets::Opconfig) -> Self {
        Self(u16::from_le_bytes(register.into()))
    }

    /// Battery insertion detected via BIN pin when set.
    #[must_use]
    pub fn battery_insertion_enable(self) -> bool {
        self.register().bie()
    }

    /// Internal weak pull-up on BIN pin enabled.
    #[must_use]
    pub fn bin_pullup_enable(self) -> bool {
        self.register().bi_pu_en()
    }

    /// GPOUT pin is active-high when set.
    #[must_use]
    pub fn gpout_active_high(self) -> bool {
        self.register().gpiopol()
    }

    /// Sleep mode may be entered when operating conditions allow.
    #[must_use]
    pub fn sleep_enable(self) -> bool {
        self.register().sleep()
    }

    /// Remaining capacity is updated from full charge capacity on charge termination.
    #[must_use]
    pub fn rmfcc(self) -> bool {
        self.register().rmfcc()
    }

    /// GPOUT drives BAT_LOW instead of SOC_INT.
    #[must_use]
    pub fn gpout_battery_low(self) -> bool {
        self.register().batlowen()
    }

    /// Host-provided temperature via `Temperature()` when set.
    #[must_use]
    pub fn temperature_from_host(self) -> bool {
        self.register().temps()
    }

    /// Enable battery insertion detection via BIN pin.
    #[must_use]
    pub fn with_battery_insertion_enable(self, enabled: bool) -> Self {
        self.with_register(set_bit_field(
            self.register(),
            |register| register.bie(),
            enabled,
            0x2000,
        ))
    }

    /// Enable sleep mode.
    #[must_use]
    pub fn with_sleep_enable(self, enabled: bool) -> Self {
        self.with_register(set_bit_field(
            self.register(),
            |register| register.sleep(),
            enabled,
            0x0020,
        ))
    }

    /// Select host-provided temperature.
    #[must_use]
    pub fn with_temperature_from_host(self, enabled: bool) -> Self {
        self.with_register(set_bit_field(
            self.register(),
            |register| register.temps(),
            enabled,
            0x0001,
        ))
    }

    /// Select GPOUT BAT_LOW function.
    #[must_use]
    pub fn with_gpout_battery_low(self, enabled: bool) -> Self {
        self.with_register(set_bit_field(
            self.register(),
            |register| register.batlowen(),
            enabled,
            0x0002,
        ))
    }
}

/// Operation configuration B register (`OpConfigB`, TRM Table 6-8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpConfigB(pub u8);

impl OpConfigB {
    /// Default value (`0x0F`).
    pub const DEFAULT: Self = Self(0x0F);

    /// SOC smoothing enabled.
    #[must_use]
    pub const fn soc_smoothing(self) -> bool {
        (self.0 & 0x04) != 0
    }

    /// Enable SOC smoothing.
    #[must_use]
    pub const fn with_soc_smoothing(self, enabled: bool) -> Self {
        Self(set_bit_u8(self.0, 0x04, enabled))
    }
}

/// Temperature safety thresholds (Safety subclass, TRM §6.4.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SafetyThresholds {
    /// Over-temperature threshold in 0.1 °C.
    pub over_temp_deci_c: i16,
    /// Under-temperature threshold in 0.1 °C.
    pub under_temp_deci_c: i16,
    /// Temperature hysteresis in 0.1 °C.
    pub temp_hys_deci_c: u8,
}

impl SafetyThresholds {
    /// TRM default values (55 °C / 0 °C / 5 °C hysteresis).
    pub const DEFAULT: Self = Self {
        over_temp_deci_c: 550,
        under_temp_deci_c: 0,
        temp_hys_deci_c: 50,
    };

    pub(crate) fn validate(self) -> Result<Self, Error<()>> {
        range_i16(
            self.over_temp_deci_c,
            -1200,
            1200,
            "over_temp_deci_c",
        )?;
        range_i16(
            self.under_temp_deci_c,
            -1200,
            1200,
            "under_temp_deci_c",
        )?;
        range_u8(self.temp_hys_deci_c, 0, 255, "temp_hys_deci_c")?;
        Ok(self)
    }
}

/// SOC alert thresholds (Discharge subclass, TRM §6.4.1.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DischargeThresholds {
    /// SOC1 set threshold (%).
    pub soc1_set_pct: u8,
    /// SOC1 clear threshold (%).
    pub soc1_clear_pct: u8,
    /// SOCF set threshold (%).
    pub socf_set_pct: u8,
    /// SOCF clear threshold (%).
    pub socf_clear_pct: u8,
}

impl DischargeThresholds {
    /// TRM default values.
    pub const DEFAULT: Self = Self {
        soc1_set_pct: 10,
        soc1_clear_pct: 15,
        socf_set_pct: 2,
        socf_clear_pct: 5,
    };

    pub(crate) fn validate(self) -> Result<Self, Error<()>> {
        range_u8(self.soc1_set_pct, 0, 100, "soc1_set_pct")?;
        range_u8(self.soc1_clear_pct, 0, 100, "soc1_clear_pct")?;
        range_u8(self.socf_set_pct, 0, 100, "socf_set_pct")?;
        range_u8(self.socf_clear_pct, 0, 100, "socf_clear_pct")?;
        Ok(self)
    }
}

/// Charge termination alarm thresholds (Charge Termination subclass, TRM §6.4.1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChargeTerminationThresholds {
    /// Terminate charge alarm set threshold (%). Use `-1` for primary charge termination.
    pub tca_set_pct: i8,
    /// Terminate charge alarm clear threshold (%).
    pub tca_clear_pct: i8,
    /// Full charge set threshold (%). Use `-1` for primary charge termination.
    pub fc_set_pct: i8,
    /// Full charge clear threshold (%).
    pub fc_clear_pct: i8,
}

impl ChargeTerminationThresholds {
    /// TRM default values.
    pub const DEFAULT: Self = Self {
        tca_set_pct: 99,
        tca_clear_pct: 95,
        fc_set_pct: -1,
        fc_clear_pct: 98,
    };

    pub(crate) fn validate(self) -> Result<Self, Error<()>> {
        range_i8(self.tca_set_pct, -1, 100, "tca_set_pct")?;
        range_i8(self.tca_clear_pct, -1, 100, "tca_clear_pct")?;
        range_i8(self.fc_set_pct, -1, 100, "fc_set_pct")?;
        range_i8(self.fc_clear_pct, 0, 100, "fc_clear_pct")?;
        Ok(self)
    }
}

/// Impedance Track Ra table for golden-file programming (Ra RAM subclass 89).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RaTable {
    /// Fifteen Ra entries (`R_a0 0` … `R_a0 14`).
    pub entries: [u16; RA_TABLE_LEN],
}

impl RaTable {
    /// Default Ra table for BQ27441-G1B (TRM Table 6-4).
    pub const G1B_DEFAULT: Self = Self {
        entries: [16, 17, 20, 24, 20, 18, 20, 20, 21, 22, 24, 31, 49, 98, 375],
    };

    /// Default Ra table for BQ27441-G1A (TRM Table 6-4).
    pub const G1A_DEFAULT: Self = Self {
        entries: [102, 102, 99, 107, 72, 59, 62, 63, 53, 47, 60, 70, 140, 369, 588],
    };

    /// Return chemistry-specific defaults.
    #[must_use]
    pub const fn for_chemistry(chem: ChemId) -> Self {
        match chem {
            ChemId::G1A => Self::G1A_DEFAULT,
            ChemId::G1B => Self::G1B_DEFAULT,
        }
    }

    pub(crate) fn validate(self) -> Result<Self, Error<()>> {
        for &entry in &self.entries {
            if entry > 32_767 {
                return Err(Error::InvalidParam);
            }
        }
        Ok(self)
    }
}

/// User-facing battery configuration snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryConfig {
    /// Design capacity in mAh.
    pub design_capacity_mah: u16,
    /// Design energy in mWh.
    pub design_energy_mwh: u16,
    /// Terminate voltage in mV.
    pub terminate_voltage_mv: u16,
    /// Taper voltage in mV.
    pub taper_voltage_mv: u16,
    /// Charge termination voltage in mV (`V at Chg Term`).
    pub charge_termination_voltage_mv: u16,
    /// Sleep current threshold in mA.
    pub sleep_current_ma: u16,
    /// Charge taper rate (0.1 Hr rate, TRM default 100 ≈ C/10).
    pub taper_rate: i16,
    /// Reserve capacity in mAh after 0% SOC (TRM default 0).
    pub reserve_cap_mah: u16,
    /// Operation configuration register.
    pub op_config: OpConfig,
    /// Operation configuration B register.
    pub op_config_b: OpConfigB,
    /// Optional current detection thresholds.
    pub current_thresholds: Option<CurrentThresholds>,
    /// Optional Update Status byte for learning / sealed defaults.
    pub update_status: Option<UpdateStatus>,
    /// Optional temperature safety thresholds.
    pub safety: Option<SafetyThresholds>,
    /// Optional SOC alert thresholds.
    pub discharge: Option<DischargeThresholds>,
    /// Optional charge termination alarm thresholds.
    pub charge_termination: Option<ChargeTerminationThresholds>,
    /// Optional Ra table for golden-file programming.
    pub ra_table: Option<RaTable>,
}

impl BatteryConfig {
    /// Default configuration for BQ27441-G1B.
    pub const G1B_DEFAULT: Self = Self {
        design_capacity_mah: 1_000,
        design_energy_mwh: 3_800,
        terminate_voltage_mv: 3_200,
        taper_voltage_mv: 4_200,
        charge_termination_voltage_mv: 4_290,
        sleep_current_ma: 10,
        taper_rate: 100,
        reserve_cap_mah: 0,
        op_config: OpConfig::DEFAULT,
        op_config_b: OpConfigB::DEFAULT,
        current_thresholds: None,
        update_status: None,
        safety: None,
        discharge: None,
        charge_termination: None,
        ra_table: None,
    };

    /// Default configuration for BQ27441-G1A.
    pub const G1A_DEFAULT: Self = Self {
        design_capacity_mah: 1_340,
        design_energy_mwh: 4_960,
        terminate_voltage_mv: 3_200,
        taper_voltage_mv: 4_100,
        charge_termination_voltage_mv: 4_190,
        sleep_current_ma: 10,
        taper_rate: 100,
        reserve_cap_mah: 0,
        op_config: OpConfig::DEFAULT,
        op_config_b: OpConfigB::DEFAULT,
        current_thresholds: None,
        update_status: None,
        safety: None,
        discharge: None,
        charge_termination: None,
        ra_table: None,
    };

    /// Return chemistry-specific defaults.
    #[must_use]
    pub const fn for_chemistry(chem: ChemId) -> Self {
        match chem {
            ChemId::G1A => Self::G1A_DEFAULT,
            ChemId::G1B => Self::G1B_DEFAULT,
        }
    }

    /// Validate all fields against TRM min/max ranges.
    pub fn validate(&self) -> Result<(), Error<()>> {
        range_u16(
            self.design_capacity_mah,
            0,
            8_000,
            "design_capacity_mah",
        )?;
        range_u16(self.design_energy_mwh, 0, 32_767, "design_energy_mwh")?;
        range_u16(
            self.terminate_voltage_mv,
            2_500,
            3_700,
            "terminate_voltage_mv",
        )?;
        range_u16(self.taper_voltage_mv, 0, 5_000, "taper_voltage_mv")?;
        range_u16(
            self.charge_termination_voltage_mv,
            0,
            5_000,
            "charge_termination_voltage_mv",
        )?;
        range_u16(self.sleep_current_ma, 0, 1_000, "sleep_current_ma")?;
        range_i16(self.taper_rate, 0, 2000, "taper_rate")?;
        range_u16(self.reserve_cap_mah, 0, 9000, "reserve_cap_mah")?;

        if let Some(thresholds) = self.current_thresholds {
            thresholds.validate_with_taper(self.taper_rate)?;
        }

        if let Some(safety) = self.safety {
            safety.validate()?;
        }
        if let Some(discharge) = self.discharge {
            discharge.validate()?;
        }
        if let Some(charge) = self.charge_termination {
            charge.validate()?;
        }
        if let Some(ra) = self.ra_table {
            ra.validate()?;
        }

        Ok(())
    }

    fn read_from_device<I, E>(device: &mut Bq27441Device<I>) -> Result<Self, Error<E>>
    where
        I: RegisterInterface<AddressType = u8, Error = E>,
        E: Debug,
    {
        select_block(device, subclass::STATE, 0)?;
        let design_capacity_mah =
            u16_le(device.dm_state_block_0().dm_design_capacity().read().map_err(Error::I2c)?);
        let design_energy_mwh =
            u16_le(device.dm_state_block_0().dm_design_energy().read().map_err(Error::I2c)?);
        let terminate_voltage_mv =
            u16_le(device.dm_state_block_0().dm_terminate_voltage().read().map_err(Error::I2c)?);
        let taper_voltage_mv =
            u16_le(device.dm_state_block_0().dm_taper_voltage().read().map_err(Error::I2c)?);
        let sleep_current_ma =
            u16::from(u8_le(device.dm_state_block_0().dm_sleep_current().read().map_err(Error::I2c)?));
        let taper_rate =
            i16_le(device.dm_state_block_0().dm_taper_rate().read().map_err(Error::I2c)?);
        let reserve_cap_mah =
            u16_le(device.dm_state_block_0().dm_reserve_cap().read().map_err(Error::I2c)?);
        let update_status = UpdateStatus(u8_le(
            device
                .dm_state_block_0()
                .dm_update_status()
                .read()
                .map_err(Error::I2c)?,
        ));

        select_block(device, subclass::STATE, 1)?;
        let charge_termination_voltage_mv =
            u16_le(device.dm_state_block_1().dm_v_at_chg_term().read().map_err(Error::I2c)?);

        select_block(device, subclass::REGISTERS, 0)?;
        let op_config = OpConfig(u16_le(
            device.dm_registers_block_0().dm_op_config().read().map_err(Error::I2c)?,
        ));
        let op_config_b = OpConfigB(u8_le(
            device
                .dm_registers_block_0()
                .dm_op_config_b()
                .read()
                .map_err(Error::I2c)?,
        ));

        select_block(device, subclass::CURRENT_THRESHOLDS, 0)?;
        let current_thresholds = CurrentThresholds {
            dsg_rate: i16_le(
                device
                    .dm_current_thresholds_block_0()
                    .dm_dsg_current()
                    .read()
                    .map_err(Error::I2c)?,
            ),
            chg_rate: i16_le(
                device
                    .dm_current_thresholds_block_0()
                    .dm_chg_current()
                    .read()
                    .map_err(Error::I2c)?,
            ),
            quit_rate: i16_le(
                device
                    .dm_current_thresholds_block_0()
                    .dm_quit_current()
                    .read()
                    .map_err(Error::I2c)?,
            ),
        };

        select_block(device, subclass::SAFETY, 0)?;
        let safety = SafetyThresholds {
            over_temp_deci_c: i16_le(
                device.dm_safety_block_0().dm_over_temp().read().map_err(Error::I2c)?,
            ),
            under_temp_deci_c: i16_le(
                device
                    .dm_safety_block_0()
                    .dm_under_temp()
                    .read()
                    .map_err(Error::I2c)?,
            ),
            temp_hys_deci_c: u8_le(
                device.dm_safety_block_0().dm_temp_hys().read().map_err(Error::I2c)?,
            ),
        };

        select_block(device, subclass::DISCHARGE, 0)?;
        let discharge = DischargeThresholds {
            soc1_set_pct: u8_le(
                device.dm_discharge_block_0().dm_soc_1_set().read().map_err(Error::I2c)?,
            ),
            soc1_clear_pct: u8_le(
                device
                    .dm_discharge_block_0()
                    .dm_soc_1_clear()
                    .read()
                    .map_err(Error::I2c)?,
            ),
            socf_set_pct: u8_le(
                device.dm_discharge_block_0().dm_socf_set().read().map_err(Error::I2c)?,
            ),
            socf_clear_pct: u8_le(
                device
                    .dm_discharge_block_0()
                    .dm_socf_clear()
                    .read()
                    .map_err(Error::I2c)?,
            ),
        };

        select_block(device, subclass::CHARGE_TERMINATION, 0)?;
        let charge_termination = ChargeTerminationThresholds {
            tca_set_pct: u8_le(
                device
                    .dm_charge_termination_block_0()
                    .dm_tca_set()
                    .read()
                    .map_err(Error::I2c)?,
            ) as i8,
            tca_clear_pct: u8_le(
                device
                    .dm_charge_termination_block_0()
                    .dm_tca_clear()
                    .read()
                    .map_err(Error::I2c)?,
            ) as i8,
            fc_set_pct: u8_le(
                device
                    .dm_charge_termination_block_0()
                    .dm_fc_set()
                    .read()
                    .map_err(Error::I2c)?,
            ) as i8,
            fc_clear_pct: u8_le(
                device
                    .dm_charge_termination_block_0()
                    .dm_fc_clear()
                    .read()
                    .map_err(Error::I2c)?,
            ) as i8,
        };

        Ok(Self {
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
            ra_table: Some(read_ra_table(device)?),
        })
    }

    fn write_to_device<I, E>(self, device: &mut Bq27441Device<I>) -> Result<(), Error<E>>
    where
        I: RegisterInterface<AddressType = u8, Error = E> + BufferInterface<AddressType = u8, Error = E>,
        E: Debug,
    {
        self.validate().map_err(|_| Error::InvalidParam)?;

        let mut session = ConfigUpdateSession::new();
        write_state_subclass(device, self, &mut session)?;
        write_registers_subclass(device, self, &mut session)?;

        if let Some(safety) = self.safety {
            safety.validate().map_err(|_| Error::InvalidParam)?;
            write_safety_subclass(device, safety, &mut session)?;
        }
        if let Some(discharge) = self.discharge {
            discharge.validate().map_err(|_| Error::InvalidParam)?;
            write_discharge_subclass(device, discharge, &mut session)?;
        }
        if let Some(charge) = self.charge_termination {
            charge.validate().map_err(|_| Error::InvalidParam)?;
            write_charge_termination_subclass(device, charge, &mut session)?;
        }
        if let Some(thresholds) = self.current_thresholds {
            thresholds
                .validate_with_taper(self.taper_rate)
                .map_err(|_| Error::InvalidParam)?;
            write_current_thresholds_subclass(device, thresholds, &mut session)?;
        }
        if let Some(ra) = self.ra_table {
            ra.validate().map_err(|_| Error::InvalidParam)?;
            write_ra_table(device, ra, &mut session)?;
        }

        session.commit_all(device)
    }
}

/// Marker error type for validation-only operations.
type ValidationResult<T> = Result<T, Error<()>>;

const fn set_bit(value: u16, mask: u16, enabled: bool) -> u16 {
    if enabled {
        value | mask
    } else {
        value & !mask
    }
}

const fn set_bit_u8(value: u8, mask: u8, enabled: bool) -> u8 {
    if enabled {
        value | mask
    } else {
        value & !mask
    }
}

fn set_bit_field(
    register: field_sets::Opconfig,
    _current: impl Fn(&field_sets::Opconfig) -> bool,
    enabled: bool,
    mask: u16,
) -> field_sets::Opconfig {
    let value = u16::from_le_bytes(register.into());
    field_sets::Opconfig::from(set_bit(value, mask, enabled).to_le_bytes())
}

fn range_u16(value: u16, min: u16, max: u16, _name: &str) -> ValidationResult<()> {
    if value < min || value > max {
        return Err(Error::InvalidParam);
    }
    Ok(())
}

fn range_u8(value: u8, min: u8, max: u8, _name: &str) -> ValidationResult<()> {
    if value < min || value > max {
        return Err(Error::InvalidParam);
    }
    Ok(())
}

fn range_i16(value: i16, min: i16, max: i16, _name: &str) -> ValidationResult<()> {
    if value < min || value > max {
        return Err(Error::InvalidParam);
    }
    Ok(())
}

fn range_i8(value: i8, min: i8, max: i8, _name: &str) -> ValidationResult<()> {
    if value < min || value > max {
        return Err(Error::InvalidParam);
    }
    Ok(())
}

fn read_ra_table<I, E>(device: &mut Bq27441Device<I>) -> Result<RaTable, Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    select_block(device, subclass::RA_RAM, 0)?;
    let mut entries = [0u16; RA_TABLE_LEN];
    for (index, entry) in entries.iter_mut().enumerate() {
        *entry = u16_le(
            device
                .dm_ra_ram_block_0()
                .dm_ra_entry(index)
                .read()
                .map_err(Error::I2c)?,
        );
    }
    Ok(RaTable { entries })
}

fn write_state_subclass<I, E>(
    device: &mut Bq27441Device<I>,
    config: BatteryConfig,
    session: &mut ConfigUpdateSession,
) -> Result<(), Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E> + BufferInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    select_block(device, subclass::STATE, 0)?;
    device
        .dm_state_block_0()
        .dm_design_capacity()
        .write(|register| {
            *register = field_sets::DmDesignCapacity::from(i16_field(config.design_capacity_mah as i16));
        })
        .map_err(Error::I2c)?;
    device
        .dm_state_block_0()
        .dm_design_energy()
        .write(|register| {
            *register = field_sets::DmDesignEnergy::from(i16_field(config.design_energy_mwh as i16));
        })
        .map_err(Error::I2c)?;
    device
        .dm_state_block_0()
        .dm_terminate_voltage()
        .write(|register| {
            *register =
                field_sets::DmTerminateVoltage::from(i16_field(config.terminate_voltage_mv as i16));
        })
        .map_err(Error::I2c)?;
    if let Some(status) = config.update_status {
        device
            .dm_state_block_0()
            .dm_update_status()
            .write(|register| *register = field_sets::DmUpdateStatus::from([status.0]))
            .map_err(Error::I2c)?;
    }
    device
        .dm_state_block_0()
        .dm_reserve_cap()
        .write(|register| {
            *register = field_sets::DmReserveCap::from(i16_field(config.reserve_cap_mah as i16));
        })
        .map_err(Error::I2c)?;
    device
        .dm_state_block_0()
        .dm_taper_rate()
        .write(|register| *register = field_sets::DmTaperRate::from(i16_field(config.taper_rate)))
        .map_err(Error::I2c)?;
    device
        .dm_state_block_0()
        .dm_taper_voltage()
        .write(|register| {
            *register = field_sets::DmTaperVoltage::from(i16_field(config.taper_voltage_mv as i16));
        })
        .map_err(Error::I2c)?;
    device
        .dm_state_block_0()
        .dm_sleep_current()
        .write(|register| {
            *register = field_sets::DmSleepCurrent::from([config.sleep_current_ma as u8]);
        })
        .map_err(Error::I2c)?;
    session.mark_block(subclass::STATE, 0);

    select_block(device, subclass::STATE, 1)?;
    device
        .dm_state_block_1()
        .dm_v_at_chg_term()
        .write(|register| {
            *register = field_sets::DmVAtChgTerm::from(i16_field(
                config.charge_termination_voltage_mv as i16,
            ));
        })
        .map_err(Error::I2c)?;
    session.mark_block(subclass::STATE, 1);
    Ok(())
}

fn write_registers_subclass<I, E>(
    device: &mut Bq27441Device<I>,
    config: BatteryConfig,
    session: &mut ConfigUpdateSession,
) -> Result<(), Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E> + BufferInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    select_block(device, subclass::REGISTERS, 0)?;
    device
        .dm_registers_block_0()
        .dm_op_config()
        .write(|register| *register = field_sets::DmOpConfig::from(u16_field(config.op_config.0)))
        .map_err(Error::I2c)?;
    device
        .dm_registers_block_0()
        .dm_op_config_b()
        .write(|register| *register = field_sets::DmOpConfigB::from([config.op_config_b.0]))
        .map_err(Error::I2c)?;
    session.mark_block(subclass::REGISTERS, 0);
    Ok(())
}

fn write_safety_subclass<I, E>(
    device: &mut Bq27441Device<I>,
    safety_cfg: SafetyThresholds,
    session: &mut ConfigUpdateSession,
) -> Result<(), Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E> + BufferInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    select_block(device, subclass::SAFETY, 0)?;
    device
        .dm_safety_block_0()
        .dm_over_temp()
        .write(|register| {
            *register = field_sets::DmOverTemp::from(i16_field(safety_cfg.over_temp_deci_c));
        })
        .map_err(Error::I2c)?;
    device
        .dm_safety_block_0()
        .dm_under_temp()
        .write(|register| {
            *register = field_sets::DmUnderTemp::from(i16_field(safety_cfg.under_temp_deci_c));
        })
        .map_err(Error::I2c)?;
    device
        .dm_safety_block_0()
        .dm_temp_hys()
        .write(|register| *register = field_sets::DmTempHys::from([safety_cfg.temp_hys_deci_c]))
        .map_err(Error::I2c)?;
    session.mark_block(subclass::SAFETY, 0);
    Ok(())
}

fn write_discharge_subclass<I, E>(
    device: &mut Bq27441Device<I>,
    discharge_cfg: DischargeThresholds,
    session: &mut ConfigUpdateSession,
) -> Result<(), Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E> + BufferInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    select_block(device, subclass::DISCHARGE, 0)?;
    device
        .dm_discharge_block_0()
        .dm_soc_1_set()
        .write(|register| *register = field_sets::DmSoc1Set::from([discharge_cfg.soc1_set_pct]))
        .map_err(Error::I2c)?;
    device
        .dm_discharge_block_0()
        .dm_soc_1_clear()
        .write(|register| {
            *register = field_sets::DmSoc1Clear::from([discharge_cfg.soc1_clear_pct]);
        })
        .map_err(Error::I2c)?;
    device
        .dm_discharge_block_0()
        .dm_socf_set()
        .write(|register| *register = field_sets::DmSocfSet::from([discharge_cfg.socf_set_pct]))
        .map_err(Error::I2c)?;
    device
        .dm_discharge_block_0()
        .dm_socf_clear()
        .write(|register| {
            *register = field_sets::DmSocfClear::from([discharge_cfg.socf_clear_pct]);
        })
        .map_err(Error::I2c)?;
    session.mark_block(subclass::DISCHARGE, 0);
    Ok(())
}

fn write_current_thresholds_subclass<I, E>(
    device: &mut Bq27441Device<I>,
    thresholds: CurrentThresholds,
    session: &mut ConfigUpdateSession,
) -> Result<(), Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E> + BufferInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    select_block(device, subclass::CURRENT_THRESHOLDS, 0)?;
    device
        .dm_current_thresholds_block_0()
        .dm_dsg_current()
        .write(|register| *register = field_sets::DmDsgCurrent::from(i16_field(thresholds.dsg_rate)))
        .map_err(Error::I2c)?;
    device
        .dm_current_thresholds_block_0()
        .dm_chg_current()
        .write(|register| *register = field_sets::DmChgCurrent::from(i16_field(thresholds.chg_rate)))
        .map_err(Error::I2c)?;
    device
        .dm_current_thresholds_block_0()
        .dm_quit_current()
        .write(|register| {
            *register = field_sets::DmQuitCurrent::from(i16_field(thresholds.quit_rate));
        })
        .map_err(Error::I2c)?;
    session.mark_block(subclass::CURRENT_THRESHOLDS, 0);
    Ok(())
}

fn write_charge_termination_subclass<I, E>(
    device: &mut Bq27441Device<I>,
    charge_cfg: ChargeTerminationThresholds,
    session: &mut ConfigUpdateSession,
) -> Result<(), Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E> + BufferInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    select_block(device, subclass::CHARGE_TERMINATION, 0)?;
    device
        .dm_charge_termination_block_0()
        .dm_tca_set()
        .write(|register| *register = field_sets::DmTcaSet::from([charge_cfg.tca_set_pct as u8]))
        .map_err(Error::I2c)?;
    device
        .dm_charge_termination_block_0()
        .dm_tca_clear()
        .write(|register| {
            *register = field_sets::DmTcaClear::from([charge_cfg.tca_clear_pct as u8]);
        })
        .map_err(Error::I2c)?;
    device
        .dm_charge_termination_block_0()
        .dm_fc_set()
        .write(|register| *register = field_sets::DmFcSet::from([charge_cfg.fc_set_pct as u8]))
        .map_err(Error::I2c)?;
    device
        .dm_charge_termination_block_0()
        .dm_fc_clear()
        .write(|register| *register = field_sets::DmFcClear::from([charge_cfg.fc_clear_pct as u8]))
        .map_err(Error::I2c)?;
    session.mark_block(subclass::CHARGE_TERMINATION, 0);
    Ok(())
}

fn write_ra_table<I, E>(
    device: &mut Bq27441Device<I>,
    ra: RaTable,
    session: &mut ConfigUpdateSession,
) -> Result<(), Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E> + BufferInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    select_block(device, subclass::RA_RAM, 0)?;
    for (index, &entry) in ra.entries.iter().enumerate() {
        device
            .dm_ra_ram_block_0()
            .dm_ra_entry(index)
            .write(|register| *register = field_sets::DmRaEntry::from(i16_field(entry as i16)))
            .map_err(Error::I2c)?;
    }
    session.mark_block(subclass::RA_RAM, 0);
    Ok(())
}

fn wait_for_config_mode<I, E, D, F>(
    device: &mut Bq27441Device<I>,
    enter: bool,
    delay: &mut D,
    mut flags_read: F,
) -> Result<(), Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E>,
    E: Debug,
    D: DelayMs,
    F: FnMut(&mut Bq27441Device<I>) -> Result<crate::field_sets::Flags, Error<E>>,
{
    let mut elapsed = 0u32;

    loop {
        let flags = flags_read(device)?;
        if flags.cfgupmode() == enter {
            return Ok(());
        }

        if elapsed >= CONFIG_TIMEOUT_MS {
            return Err(Error::ConfigModeTimeout);
        }

        delay.delay_ms(CONFIG_POLL_INTERVAL_MS);
        elapsed += CONFIG_POLL_INTERVAL_MS;
    }
}

fn read_battery_config<I, E>(device: &mut Bq27441Device<I>) -> Result<BatteryConfig, Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    BatteryConfig::read_from_device(device)
}

fn apply_battery_config<I, E>(
    device: &mut Bq27441Device<I>,
    config: &BatteryConfig,
) -> Result<(), Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E> + BufferInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    config.write_to_device(device)
}

fn design_capacity<I, E>(device: &mut Bq27441Device<I>) -> Result<u16, Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    read_design_capacity(device)
}

fn op_config<I, E>(device: &mut Bq27441Device<I>) -> Result<OpConfig, Error<E>>
where
    I: RegisterInterface<AddressType = u8, Error = E>,
    E: Debug,
{
    read_opconfig(device).map(|register| OpConfig(u16_le(register)))
}


// Blocking and async driver configuration methods.
use crate::Bq27441;
#[cfg(feature = "async")]
use crate::Bq27441Async;
use embedded_hal as hal;
#[cfg(feature = "async")]
use embedded_hal_async as hal_async;

impl<I2C> Bq27441<I2C>
where
    I2C: hal::i2c::I2c,
    I2C::Error: Debug,
{
    /// Read `CONTROL_STATUS` via the control subcommand.
    pub fn control_status(&mut self) -> Result<ControlStatus, Error<I2C::Error>> {
        Ok(ControlStatus {
            raw: self.control_read(ControlCmd::ControlStatus)?,
        })
    }

    /// Return `true` when the device is in SEALED access mode.
    pub fn is_sealed(&mut self) -> Result<bool, Error<I2C::Error>> {
        Ok(self.control_status()?.sealed())
    }

    /// Poll `Flags()[CFGUPMODE]` until it matches `enter`.
    pub fn wait_for_config_mode<D: DelayMs>(
        &mut self,
        enter: bool,
        delay: &mut D,
    ) -> Result<(), Error<I2C::Error>> {
        wait_for_config_mode(&mut self.device, enter, delay, |device| {
            device.flags().read().map_err(Error::I2c)
        })
    }

    /// Read design capacity via the extended command shortcut (works when sealed).
    pub fn design_capacity(&mut self) -> Result<u16, Error<I2C::Error>> {
        design_capacity(&mut self.device)
    }

    /// Read `OpConfig` via the extended command shortcut (works when sealed).
    pub fn op_config(&mut self) -> Result<OpConfig, Error<I2C::Error>> {
        op_config(&mut self.device)
    }

    /// Read battery configuration from Data Memory (device must be unsealed).
    pub fn read_battery_config(&mut self) -> Result<BatteryConfig, Error<I2C::Error>> {
        if self.is_sealed()? {
            return Err(Error::Sealed);
        }
        read_battery_config(&mut self.device)
    }

    /// Apply configuration while already in CONFIG UPDATE mode.
    pub fn apply_battery_config(
        &mut self,
        config: &BatteryConfig,
    ) -> Result<(), Error<I2C::Error>> {
        let flags = self.device.flags().read().map_err(Error::I2c)?;
        if !flags.cfgupmode() {
            return Err(Error::NotInConfigMode);
        }
        apply_battery_config(&mut self.device, config)
    }

    /// Full configuration workflow: unseal, enter config mode, apply, exit, reseal.
    pub fn configure_battery<D: DelayMs>(
        &mut self,
        config: &BatteryConfig,
        options: ConfigureOptions,
        delay: &mut D,
    ) -> Result<(), Error<I2C::Error>> {
        config.validate().map_err(|_| Error::InvalidParam)?;

        if options.unseal {
            if self.is_sealed()? {
                self.unseal()?;
            }
        } else if self.is_sealed()? {
            return Err(Error::Sealed);
        }

        self.enter_config_mode()?;
        self.wait_for_config_mode(true, delay)?;
        self.apply_battery_config(config)?;
        self.exit_config(options.exit)?;
        self.wait_for_config_mode(false, delay)?;

        if options.reseal {
            self.seal()?;
        }

        Ok(())
    }

    /// Exit CONFIG UPDATE mode using the selected strategy.
    pub fn exit_config(&mut self, exit: ConfigExit) -> Result<(), Error<I2C::Error>> {
        let cmd = match exit {
            ConfigExit::SoftReset => ControlCmd::SoftReset,
            ConfigExit::ExitResim => ControlCmd::ExitResim,
            ConfigExit::ExitNoResim => ControlCmd::ExitCfgUpdate,
        };
        self.control_write(cmd)
    }

    /// Convenience wrapper using [`BusyWait`] polling.
    pub fn configure_battery_default(
        &mut self,
        config: &BatteryConfig,
        options: ConfigureOptions,
    ) -> Result<(), Error<I2C::Error>> {
        let mut delay = BusyWait;
        self.configure_battery(config, options, &mut delay)
    }
}

#[cfg(feature = "async")]
impl<I2C> Bq27441Async<I2C>
where
    I2C: hal_async::i2c::I2c,
    I2C::Error: Debug,
{
    /// Read `CONTROL_STATUS` via the control subcommand.
    pub async fn control_status(&mut self) -> Result<ControlStatus, Error<I2C::Error>> {
        Ok(ControlStatus {
            raw: self.control_read(ControlCmd::ControlStatus).await?,
        })
    }

    /// Return `true` when the device is in SEALED access mode.
    pub async fn is_sealed(&mut self) -> Result<bool, Error<I2C::Error>> {
        Ok(self.control_status().await?.sealed())
    }

    /// Poll `Flags()[CFGUPMODE]` until it matches `enter`.
    pub async fn wait_for_config_mode<D: DelayMsAsync>(
        &mut self,
        enter: bool,
        delay: &mut D,
    ) -> Result<(), Error<I2C::Error>> {
        let mut elapsed = 0u32;

        loop {
            let flags = self.device.flags().read_async().await.map_err(Error::I2c)?;
            if flags.cfgupmode() == enter {
                return Ok(());
            }

            if elapsed >= CONFIG_TIMEOUT_MS {
                return Err(Error::ConfigModeTimeout);
            }

            delay.delay_ms(CONFIG_POLL_INTERVAL_MS).await;
            elapsed += CONFIG_POLL_INTERVAL_MS;
        }
    }

    /// Read design capacity via the extended command shortcut (works when sealed).
    pub async fn design_capacity(&mut self) -> Result<u16, Error<I2C::Error>> {
        crate::config_async::design_capacity(&mut self.device).await
    }

    /// Read `OpConfig` via the extended command shortcut (works when sealed).
    pub async fn op_config(&mut self) -> Result<OpConfig, Error<I2C::Error>> {
        crate::config_async::op_config(&mut self.device).await
    }

    /// Read battery configuration from Data Memory (device must be unsealed).
    pub async fn read_battery_config(&mut self) -> Result<BatteryConfig, Error<I2C::Error>> {
        if self.is_sealed().await? {
            return Err(Error::Sealed);
        }
        crate::config_async::read_battery_config(&mut self.device).await
    }

    /// Apply configuration while already in CONFIG UPDATE mode.
    pub async fn apply_battery_config(
        &mut self,
        config: &BatteryConfig,
    ) -> Result<(), Error<I2C::Error>> {
        let flags = self
            .device
            .flags()
            .read_async()
            .await
            .map_err(Error::I2c)?;
        if !flags.cfgupmode() {
            return Err(Error::NotInConfigMode);
        }
        crate::config_async::apply_battery_config(&mut self.device, config).await
    }

    /// Full configuration workflow: unseal, enter config mode, apply, exit, reseal.
    pub async fn configure_battery<D: DelayMsAsync>(
        &mut self,
        config: &BatteryConfig,
        options: ConfigureOptions,
        delay: &mut D,
    ) -> Result<(), Error<I2C::Error>> {
        config.validate().map_err(|_| Error::InvalidParam)?;

        if options.unseal {
            if self.is_sealed().await? {
                self.unseal().await?;
            }
        } else if self.is_sealed().await? {
            return Err(Error::Sealed);
        }

        self.enter_config_mode().await?;
        self.wait_for_config_mode(true, delay).await?;
        self.apply_battery_config(config).await?;
        self.exit_config(options.exit).await?;
        self.wait_for_config_mode(false, delay).await?;

        if options.reseal {
            self.seal().await?;
        }

        Ok(())
    }

    /// Exit CONFIG UPDATE mode using the selected strategy.
    pub async fn exit_config(&mut self, exit: ConfigExit) -> Result<(), Error<I2C::Error>> {
        let cmd = match exit {
            ConfigExit::SoftReset => ControlCmd::SoftReset,
            ConfigExit::ExitResim => ControlCmd::ExitResim,
            ConfigExit::ExitNoResim => ControlCmd::ExitCfgUpdate,
        };
        self.control_write(cmd).await
    }

    /// Convenience wrapper using [`BusyWait`] polling.
    pub async fn configure_battery_default(
        &mut self,
        config: &BatteryConfig,
        options: ConfigureOptions,
    ) -> Result<(), Error<I2C::Error>> {
        let mut delay = BusyWait;
        self.configure_battery(config, options, &mut delay).await
    }
}
