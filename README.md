# Requirements

- A Raspberry Pi Pico 2
- [picotool](https://github.com/raspberrypi/picotool)
- [Rust](https://rust-lang.org/)

# Running

1. Connect your Raspberry Pi Pico 2 to your computer in BOOTSEL mode.
2. Run `cargo run --release`.
3. Run `screen /dev/tty.XXX 9600`.
4. Whatever you type into the screen session will be echoed back to you.
