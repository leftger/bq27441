//! BQ27441 battery fuel gauge example for STM32WBA65RI using Embassy.
//!
//! This example demonstrates basic battery monitoring using the BQ27441 fuel gauge
//! connected via I2C to an STM32WBA65RI microcontroller.
//!
//! # Hardware Setup
//!
//! - STM32WBA65RI board
//! - BQ27441 fuel gauge connected to I2C1
//!   - SDA: PB7
//!   - SCL: PB6
//!   - I2C address: 0x55 (default)
//!
//! # Features
//!
//! This example reads and displays:
//! - Battery voltage (mV)
//! - State of charge (%)
//! - Remaining capacity (mAh)
//! - Average current (mA)
//! - Temperature (°C)
//! - Charging/discharging status

#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::i2c::{self, I2c};
use embassy_stm32::{bind_interrupts, peripherals};
use embassy_time::Timer;
use {defmt_rtt as _, panic_probe as _};

use bq27441::Bq27441Async;

bind_interrupts!(struct Irqs {
    I2C1_EV => i2c::EventInterruptHandler<peripherals::I2C1>;
    I2C1_ER => i2c::ErrorInterruptHandler<peripherals::I2C1>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("BQ27441 Battery Monitor Example");

    // Initialize STM32 peripherals
    let p = embassy_stm32::init(Default::default());

    info!("Initializing I2C...");

    // Configure I2C1 with 100 kHz standard mode
    // PB6 = SCL, PB7 = SDA
    let i2c = I2c::new(
        p.I2C1,
        p.PB6,  // SCL
        p.PB7,  // SDA
        Irqs,
        p.DMA1_CH0,
        p.DMA1_CH1,
        embassy_stm32::time::Hertz(100_000),
        Default::default(),
    );

    info!("Initializing BQ27441...");

    // Create BQ27441 driver
    let mut gauge = match Bq27441Async::new(i2c).await {
        Ok(g) => {
            info!("BQ27441 initialized successfully");
            g
        }
        Err(e) => {
            error!("Failed to initialize BQ27441: {:?}", e);
            loop {
                Timer::after_secs(1).await;
            }
        }
    };

    // Read and display device information
    match gauge.firmware_version().await {
        Ok(fw) => info!("Firmware version: 0x{:04X}", fw),
        Err(e) => error!("Failed to read firmware version: {:?}", e),
    }

    match gauge.chemistry_id().await {
        Ok(chem) => info!("Chemistry ID: 0x{:04X}", chem),
        Err(e) => error!("Failed to read chemistry ID: {:?}", e),
    }

    info!("Starting battery monitoring loop...");

    // Main monitoring loop
    loop {
        // Read battery voltage
        match gauge.voltage().await {
            Ok(voltage) => info!("Voltage: {} mV", voltage),
            Err(e) => error!("Failed to read voltage: {:?}", e),
        }

        // Read state of charge
        match gauge.state_of_charge().await {
            Ok(soc) => info!("State of Charge: {}%", soc),
            Err(e) => error!("Failed to read SOC: {:?}", e),
        }

        // Read remaining capacity
        match gauge.remaining_capacity().await {
            Ok(capacity) => info!("Remaining Capacity: {} mAh", capacity),
            Err(e) => error!("Failed to read capacity: {:?}", e),
        }

        // Read full charge capacity
        match gauge.full_charge_capacity().await {
            Ok(fcc) => info!("Full Charge Capacity: {} mAh", fcc),
            Err(e) => error!("Failed to read FCC: {:?}", e),
        }

        // Read average current
        match gauge.average_current().await {
            Ok(current) => {
                if current < 0 {
                    info!("Current: {} mA (discharging)", current);
                } else {
                    info!("Current: {} mA (charging)", current);
                }
            }
            Err(e) => error!("Failed to read current: {:?}", e),
        }

        // Read temperature
        match gauge.temperature_celsius().await {
            Ok(temp) => info!("Temperature: {:.1}°C", temp),
            Err(e) => error!("Failed to read temperature: {:?}", e),
        }

        // Read charging status
        match gauge.is_charging().await {
            Ok(charging) => {
                if charging {
                    info!("Status: Charging");
                }
            }
            Err(e) => error!("Failed to read charging status: {:?}", e),
        }

        // Check if fully charged
        match gauge.is_full_charged().await {
            Ok(full) => {
                if full {
                    info!("Battery is fully charged!");
                }
            }
            Err(e) => error!("Failed to check full charge status: {:?}", e),
        }

        info!("---");

        // Wait 5 seconds before next reading
        Timer::after_secs(5).await;
    }
}
