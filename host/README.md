This crate contains both a library for interfacing with the controller hardware (pressure gauge, turbo molecular pump, etc.) and a binary for doing the same via a TUI.

# Requirements

- [libusb](https://libusb.info/), e.g. via `brew install libusb` on macOS
- [Rust](https://rust-lang.org/)

# Running

1. Run `cargo run -p characteriser -- <device path> <ADC gauge number> <TMP address>`, e.g. `cargo run -p characteriser -- /dev/tty.usbmodem11201 1 001`.
