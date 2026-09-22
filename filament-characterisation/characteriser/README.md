A Ratatui application that drives the vacuum rig and steps an operator through characterising a tungsten filament.

See [`docs/DESIGN.md`](docs/DESIGN.md) for the full specification.

# Requirements

- [Rust](https://rust-lang.org/)
- [Python 3](https://www.python.org/)

Statistics and uncertainty propagation are done by the scripts in [`python`](python), which need a virtual environment beside them. Create it once, from this directory:

```
python3 -m venv python/.venv
python/.venv/bin/pip install -r python/requirements.txt
```

The application checks for it at start-up and refuses to run without it, printing these same two commands.

# Running

1. Run `cargo run -- <device path> <ADC gauge number> <TMP address>`, e.g. `cargo run -- /dev/tty.usbmodem11201 1 001`.

To run against simulated hardware instead of the rig, run `cargo run -- --mock`.

Diagnostics are written to `out/app.log` rather than the terminal. As with the vacuum control binary, the log level comes from `RUST_LOG`, which defaults to `error` — so run with `RUST_LOG=info` (or `debug`) to get anything useful in the file.
