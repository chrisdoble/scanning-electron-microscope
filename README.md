# Requirements

- A Raspberry Pi Debug Probe
- A Raspberry Pi Pico 2
- [probe-rs](https://probe.rs/)
- [Rust](https://rust-lang.org/)

# Running

1. Connect your Raspberry Pi Debug Probe and Pico 2 to each other and your computer.
2. Run `cargo run --release`.
3. Run `screen /dev/tty.XXX` where `XXX` identifies the Raspberry Pi Pico 2.
4. You will see log output.
