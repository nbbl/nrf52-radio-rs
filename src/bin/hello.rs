#![no_main]
#![no_std]

use nrf52_radio_rs as _;

#[cortex_m_rt::entry]
fn main() -> ! {
    defmt::println!("Hello, world!");

    nrf52_radio_rs::exit()
}
