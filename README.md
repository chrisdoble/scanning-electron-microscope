This repository contains my DIY scanning electron microscope (SEM) project.

I'm currently building a vacuum system along with a device to control it from a computer. This consists of:

- an Edwards WRG-S wide range pressure gauge,
- an Edwards ADC MkII gauge controller,
- a Pfeiffer TC-600 turbomolecular pump controller,
- a Preiffer TMH 071 P turbomolecular pump,
- a custom PCB for the controller, and
- various Rust crates for the controller.

The repository structure is as follows:

- [`case`](case): 3D models of the controller case for 3D printing.
- [`common`](common): A Rust crate containing code common to `firmware` and `host`.
- [`electronics`](electronics): KiCad files for the controller PCB.
- [`firmware`](firmware): A Rust crate containing the [Embassy](https://github.com/embassy-rs/embassy)-based firmware for the controller.
- [`host`](host): A Rust crate containing code to interface with the controller from a host computer.

# Requirements

## Hardware

- A [Raspberry Pi Debug Probe](https://www.raspberrypi.com/documentation/microcontrollers/debug-probe.html)
- A [Raspberry Pi Pico 2](https://www.raspberrypi.com/products/raspberry-pi-pico-2/)
- A [UART-to-RS-232 converter](https://core-electronics.com.au/rs232-to-serial-converter.html)
- A [UART-to-RS-485 converter](https://core-electronics.com.au/ttl-uart-to-rs485-converter-module.html)
- The PCB detailed in the KiCad files (or equivalent)
- The vacuum equipment mentioned above
- Various resistors as detailed in the KiCad schematic

## Software

- [probe-rs](https://probe.rs/)
- [Rust](https://rust-lang.org/)

# Developing

Unfortunately Cargo doesn't read member crates' `.cargo/config.toml` files when run from the workspace root. This means that, among other things, their `target`s aren't set and thus the build fails. You must to run Cargo from within each crate's directory.

## `firmware`

1. Connect the Pi Debug Probe and Pico 2 to your computer.
1. Run

   ```console
   $ cd firmware
   $ cargo run
   ```

   This uploads the firmware to the Pico 2 via the Debug Probe, runs it, and prints subsequent log messages. You don't need to run this again unless you change the firmware or want to see log messages.

## `host`

1. Run

   ```console
   $ cd host
   $ cargo run
   ```
