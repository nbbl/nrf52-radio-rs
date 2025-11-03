#! /bin/bash

# Script for flashing firmware via DFU using adafruit-nrfutil.
# Uses ELF built from hello.rs example:
# $ cargo build --bin hello
# $ ./flash-dfu.sh /dev/tty.<mounted-board-name>
# for example
# $ ./flash-dfu.sh /dev/tty.usbmodem1301

set -e

arm-none-eabi-objcopy \
    -O ihex \
    target/thumbv7em-none-eabihf/debug/hello \
    hello.hex

adafruit-nrfutil \
    dfu genpkg \
    --dev-type 0x0052 \
    --sd-req 0x0123 \
    --application hello.hex hello.zip

adafruit-nrfutil \
    --verbose \
    dfu serial \
    -pkg hello.zip \
    -p $0 \
    -b 115200 \
    --singlebank