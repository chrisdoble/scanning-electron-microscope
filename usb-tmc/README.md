A small async Rust library for communicating with a single USBTMC (USB Test
and Measurement Class) instrument — e.g. an oscilloscope or power supply —
over its bulk endpoints, built on [`rusb`].

See [`docs/API.md`](docs/API.md) for the full API design.

[`rusb`]: https://docs.rs/rusb

# Requirements

- [Rust](https://rust-lang.org/)
- [libusb](https://libusb.info/), e.g. via `brew install libusb` on macOS

# Features

- Find a device by USB vendor/product ID, open it, and locate its USBTMC
  interface and bulk endpoints.
- Send a message to the device (`DEV_DEP_MSG_OUT`).
- Request a response and read it back in full
  (`REQUEST_DEV_DEP_MSG_IN`/`DEV_DEP_MSG_IN`), reassembling multi-transfer
  responses using the End-Of-Message (EOM) bit.

# Limitations

- No support for the control-endpoint requests used to abort or recover a
  stuck transfer — if the host and device get out of sync, drop and reopen
  the `UsbTmcDevice`.
- `read` relies solely on the EOM bit; `TermChar`-based early termination
  isn't supported.
- If multiple devices match the given vendor/product ID, `open` returns an
  error rather than picking one.
- Targets macOS — `open` doesn't attempt to detach a kernel driver before
  claiming the interface.
