A Ratatui application that drives the vacuum rig and steps an operator through characterising a tungsten filament.

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for the full specification.

# Requirements

- [Rust](https://rust-lang.org/)
- [uv](https://docs.astral.sh/uv/)

Statistics and uncertainty propagation are done by the scripts in [`python`](python), which need a virtual environment beside them. The Python version is pinned in [`python/.python-version`](python/.python-version). Create the environment once, from this directory, with uv, which reads that file and downloads the pinned Python if it isn't installed:

```
uv venv --directory python
uv pip install --directory python -r requirements.txt
```

The application checks for it at start-up and refuses to run without it, printing these same two commands.

# Running

1. Run `cargo run -- <device path> <ADC gauge number> <TMP address>`, e.g. `cargo run -- /dev/tty.usbmodem11201 1 001`.

To run against simulated hardware instead of the rig, run `cargo run -- --mock`.

Diagnostics are written to `out/app.log` rather than the terminal. As with the vacuum control binary, the log level comes from `RUST_LOG`, which defaults to `error` — so run with `RUST_LOG=info` (or `debug`) to get anything useful in the file.
