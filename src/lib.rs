#![no_main]
#![no_std]

use defmt_rtt as _;
use embassy_nrf::{
    Peri,
    peripherals::{RNG, TIMER0},
};
use panic_probe as _;

pub mod bsp {
    pub mod ble;
}

// TODO: Move Board into bsp module?:
pub struct Board {
    /// TIMER0 peripheral
    pub timer0: Peri<'static, TIMER0>,
    /// Random number generator
    pub rng: Peri<'static, RNG>,
    /// Bluetooth Low Energy
    pub ble: bsp::ble::BleControllerBuilder<'static>,
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
            timer0: p.TIMER0,
            rng: p.RNG,
            ble: bsp::ble::BleControllerBuilder::new(
                p.RTC0, p.TEMP, p.PPI_CH17, p.PPI_CH18, p.PPI_CH19, p.PPI_CH20, p.PPI_CH21,
                p.PPI_CH22, p.PPI_CH23, p.PPI_CH24, p.PPI_CH25, p.PPI_CH26, p.PPI_CH27, p.PPI_CH28,
                p.PPI_CH29, p.PPI_CH30, p.PPI_CH31,
            ),
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
