# Requirements

- A Raspberry Pi Debug Probe
- A Raspberry Pi Pico 2
- [probe-rs](https://probe.rs/)
- [Rust](https://rust-lang.org/)

# Developing

Unfortunately Cargo doesn't read member crates' `.cargo/config.toml` files when run from the workspace root. This means that, among other things, their `target`s aren't set and thus the build fails. You must to run Cargo from within each crate's directory.

```console
$ cd firmware
$ cargo run
```
