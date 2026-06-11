This repository contains my DIY scanning electron microscope project.

My initial goal is to image samples using secondary electron emission and control the microscope from my computer.

I'm also documenting the process on YouTube [here](https://www.youtube.com/playlist?list=PLmlXFuUXRl5AR0MqcsTuL44zxy5YJyGKN).

# Roadmap

- [x] Purchase vacuum equipment
  - Edwards ADC MkII gauge controller
  - Edwards WRG-S wide range pressure gauge
  - Pfeiffer TC-600 turbomolecular pump controller
  - Pfeiffer TMH 071 P turbomolecular pump
- [x] Build a controller for the vacuum equipment
- [ ] Build an electron gun
- [ ] Build electrostatic or electromagnetic lenses
- [ ] Build an Everhart-Thornley detector
- [ ] ???
- [ ] Look at stuff

# Repository structure

The repository structure is as follows:

- [`case`](case): 3D models of the controller case for 3D printing.
- [`common`](common): A Rust crate containing code common to `firmware` and `host`.
- [`electron-gun-simulation`](electron-gun-simulation): A web-based simulation of an electron gun.
- [`electronics`](electronics): KiCad files for the controller PCB.
- [`firmware`](firmware): A Rust crate containing the [Embassy](https://github.com/embassy-rs/embassy)-based firmware for the controller.
- [`host`](host): A Rust crate containing code to interface with the controller from a host computer.
- [`usb-tmc`](usb-tmc): A Rust crate containing an implementation of the [USB Test and Measurement Class](https://www.usb.org/document-library/test-measurement-class-specification) for interfacing with oscilloscopes, power supplies, etc.

# Required hardware

- The PCB detailed in the KiCad files including
  - [Raspberry Pi Debug Probe](https://www.raspberrypi.com/documentation/microcontrollers/debug-probe.html)
  - [Raspberry Pi Pico 2](https://www.raspberrypi.com/products/raspberry-pi-pico-2/)
  - [UART-to-RS-232 converter](https://core-electronics.com.au/rs232-to-serial-converter.html)
  - [UART-to-RS-485 converter](https://core-electronics.com.au/ttl-uart-to-rs485-converter-module.html)
  - Various headers and resistors
- The vacuum equipment mentioned above

# Running

See each directory's `README.md` for instructions.
