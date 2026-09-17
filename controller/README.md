This directory contains files relating to the controller hardware that enables communication with the vacuum equipment, controls relays, etc.

# Directory structure

- [`case`](case): 3D models of the controller case for 3D printing.
- [`electronics`](electronics): KiCad files for the controller PCB.
- [`firmware`](firmware): A Rust crate containing the [Embassy](https://github.com/embassy-rs/embassy)-based firmware for the controller.

# Required hardware

- The PCB detailed in the KiCad files including
  - [Raspberry Pi Debug Probe](https://www.raspberrypi.com/documentation/microcontrollers/debug-probe.html)
  - [Raspberry Pi Pico 2](https://www.raspberrypi.com/products/raspberry-pi-pico-2/)
  - [UART-to-RS-232 converter](https://core-electronics.com.au/rs232-to-serial-converter.html)
  - [UART-to-RS-485 converter](https://core-electronics.com.au/ttl-uart-to-rs485-converter-module.html)
  - Various headers and resistors
- 2 x [SPDT relays](https://core-electronics.com.au/gravity-digital-relay-module-arduino-and-raspberry-pi-compatible.html) for reversing filament heating current
- 6 x [Banana sockets](https://www.jaycar.com.au/red-deluxe-binding-post/p/PT0453)
