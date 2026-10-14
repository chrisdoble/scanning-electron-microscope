This directory contains files relating to the hardware and software used to characterise tungsten filaments to be used in the SEM.

# Directory structure

- [`characteriser`](characteriser): A Rust crate containing the TUI that steps an operator through characterising a filament.
- [`docs`](docs): Design documents for the characteriser and visualiser, numbered in the order they were written, starting with the architecture in [`1_ARCHITECTURE.md`](docs/1_ARCHITECTURE.md).
- [`mount`](mount): 3D models of the filament mount.
- [`visualiser`](visualiser): A local web page that plots a results file written by the characteriser, updating as the file changes.
