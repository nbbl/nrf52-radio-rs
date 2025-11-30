#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_nrf::{
    bind_interrupts, peripherals,
    twim::{self},
};
use embedded_graphics::{
    mono_font::{MonoTextStyleBuilder, ascii::FONT_6X10},
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Baseline, Text},
};
use nrf52_radio_rs::Board;
use ssd1306_i2c::{Builder, prelude::*};

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    bind_interrupts!(struct Irqs {
            TWISPI0  => twim::InterruptHandler<peripherals::TWISPI0>;
    });

    let board = Board::default();

    let twim = twim::Twim::new(
        board.twispi0,
        Irqs,
        board.p0_06,
        board.p0_05,
        Default::default(),
        &mut [],
    );

    let mut display: GraphicsMode<_> = Builder::new()
        .with_size(DisplaySize::Display128x64)
        .with_i2c_addr(0x3d)
        .with_rotation(DisplayRotation::Rotate0)
        .connect_i2c(twim)
        .into();

    display.init().unwrap();
    display.flush().unwrap();
    display.clear();

    let text_style = MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(BinaryColor::On)
        .build();

    Text::with_baseline("Hey sexy!", Point::zero(), text_style, Baseline::Top)
        .draw(&mut display)
        .unwrap();

    Text::with_baseline("Hello Rust!", Point::new(0, 16), text_style, Baseline::Top)
        .draw(&mut display)
        .unwrap();

    display.flush().unwrap();
    loop {}
}
