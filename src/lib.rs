#![no_main]
#![no_std]

use defmt_rtt as _;
use embassy_nrf::{
    Peri,
    peripherals::{P0_05, P0_06, RNG, TIMER0, TWISPI0, UARTE0},
};
use panic_probe as _;

pub mod bsp {
    pub mod ble;
}

// TODO: Move Board into bsp module?:
// TODO: Separate board structs for Adafruit and Wio Tracker L1
pub struct Board {
    /// GPIO 0.05 (OLED I2C SCL on Wio Tracker L1)
    pub p0_05: Peri<'static, P0_05>,
    /// GPIO 0.06 (OLED I2C SDA on Wio Tracker L1)
    pub p0_06: Peri<'static, P0_06>,
    /// TIMER0 peripheral
    pub timer0: Peri<'static, TIMER0>,
    /// Random number generator
    pub rng: Peri<'static, RNG>,
    /// Bluetooth Low Energy
    pub ble: bsp::ble::BleControllerBuilder<'static>,
    /// Two-Wire & Serial Peripheral Interface 0 (shared)
    pub twispi0: Peri<'static, TWISPI0>,
    // TODO: documentation.
    pub uarte0: Peri<'static, UARTE0>,
}

impl Default for Board {
    fn default() -> Self {
        Self::new(Default::default())
    }
}

impl Board {
    pub fn new(config: embassy_nrf::config::Config) -> Self {
        let p = embassy_nrf::init(config);
        Self {
            ble: bsp::ble::BleControllerBuilder::new(
                p.RTC0, p.TEMP, p.PPI_CH17, p.PPI_CH18, p.PPI_CH19, p.PPI_CH20, p.PPI_CH21,
                p.PPI_CH22, p.PPI_CH23, p.PPI_CH24, p.PPI_CH25, p.PPI_CH26, p.PPI_CH27, p.PPI_CH28,
                p.PPI_CH29, p.PPI_CH30, p.PPI_CH31,
            ),
            p0_05: p.P0_05,
            p0_06: p.P0_06,
            rng: p.RNG,
            timer0: p.TIMER0,
            twispi0: p.TWISPI0,
            uarte0: p.UARTE0
        }
    }
}

#[defmt::panic_handler]
fn panic() -> ! {
    // same panicking *behavior* as `panic-probe` but doesn't print a panic message
    // this prevents the panic message being printed *twice* when `defmt::panic` is invoked
    cortex_m::asm::udf()
}

/// Terminates the application and makes a semihosting-capable debug tool exit
/// with status code 0.
pub fn exit() -> ! {
    semihosting::process::exit(0);
}

/// Hardfault handler.
///
/// Terminates the application and makes a semihosting-capable debug tool exit
/// with an error. This seems better than the default, which is to spin in a
/// loop.
#[cortex_m_rt::exception]
unsafe fn HardFault(_frame: &cortex_m_rt::ExceptionFrame) -> ! {
    semihosting::process::exit(1);
}

// defmt-test 0.3.0 has the limitation that this `#[tests]` attribute can only be used
// once within a crate. the module can be in any file but there can only be at most
// one `#[tests]` module in this library crate
#[cfg(test)]
#[defmt_test::tests]
mod unit_tests {
    use defmt::assert;

    #[test]
    fn it_works() {
        assert!(true)
    }
}
