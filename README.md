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
- [ ] Build a filament characterisation system
- [ ] Build an electron gun
- [ ] Build electrostatic or electromagnetic lenses
- [ ] Build an Everhart-Thornley detector
- [ ] ???
- [ ] Look at stuff

# Repository structure

The repository structure is as follows:

- [`controller`](controller): This directory contains files relating to the controller hardware that enables communication with the vacuum equipment, controls relays, etc.
- [`common`](common): A Rust crate containing code common to `firmware` and `host`.
- [`electron-gun-simulation`](electron-gun-simulation): A web-based simulation of an electron gun.
- [`host`](host): A Rust crate containing both a library for interfacing with the controller hardware (pressure gauge, turbo molecular pump, etc.) and a binary for doing the same via a TUI.
- [`usb-tmc`](usb-tmc): A Rust crate containing an implementation of the [USB Test and Measurement Class](https://www.usb.org/document-library/test-measurement-class-specification) for interfacing with oscilloscopes, power supplies, etc.

# Required hardware

- A [Rigol DHO-814 oscilloscope](https://core-electronics.com.au/rigol-dho-814-oscilloscope.html)
- A [Rigol DP-932E power supply](https://core-electronics.com.au/rigol-dp-932e-triple-output-dc-power-supply.html)
- The custom controller as detailed in the [`controller`](controller) directory.
- The vacuum equipment mentioned above

# Running

See each directory's `README.md` for instructions.
