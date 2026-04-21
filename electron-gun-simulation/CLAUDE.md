# SEM Electron Gun Simulator

A web-based simulation of a thermionic electron gun for a scanning electron microscope. The user configures the gun geometry and voltages via sliders, and the app solves for the electrostatic potential field and electric field in real time.

The physics is a 2D axisymmetric Laplace solver (cylindrical coordinates, charge-free). The solver is written in Rust and compiled to WebAssembly for performance. The UI is a Vite-based TypeScript app.

## Project layout

```
sem-gun-sim/
├── docs/
│   ├── PHYSICS.md       — Governing equations, discretisation, stencils, solver algorithm.
│   │                      Read before modifying anything in crates/solver/.
│   ├── ARCHITECTURE.md  — Crate layout, type definitions, wasm boundary, data flow.
│   │                      Read before adding modules, changing interfaces, or restructuring.
│   └── UI.md            — Component structure, slider behaviour, visualisation.
│                          Read before modifying web/.
├── crates/
│   ├── Cargo.toml       — Workspace root for the two Rust crates.
│   ├── solver/          — Pure Rust Laplace solver. No wasm dependencies.
│   └── wasm-api/        — Thin wasm-bindgen wrapper. Depends on solver.
└── web/                 — Vite + TypeScript application.
```

## Build and test commands

```bash
# Run solver tests (fast, native — the primary development loop for physics)
cargo test -p solver --manifest-path crates/Cargo.toml

# Build the wasm package
wasm-pack build --target web crates/wasm-api

# Watch + rebuild wasm on Rust changes
cargo watch -s 'wasm-pack build --target web crates/wasm-api'

# Run the web dev server
cd web && pnpm run dev

# Production build
cd web && pnpm run build
```

## Validation after changes

After modifying Rust code in `crates/solver/`:
```bash
cargo clippy -p solver --manifest-path crates/Cargo.toml -- -D warnings
cargo test -p solver --manifest-path crates/Cargo.toml
```

After modifying Rust code in `crates/wasm-api/`:
```bash
cargo clippy -p wasm-api --manifest-path crates/Cargo.toml --target wasm32-unknown-unknown -- -D warnings
wasm-pack build --target web crates/wasm-api
```

After modifying TypeScript code in `web/`:
```bash
cd web && pnpm exec tsc --noEmit
```

Always fix clippy warnings and type errors before moving on. Do not suppress warnings with `#[allow(...)]` unless there is a specific documented reason.

## Critical rules

- **No cross-layer leakage.** `solver` must not know about electron guns, filaments, Wehnelts, or anodes — it only sees grids and masks. `wasm-api` must not contain numerical algorithms — it only converts GunParameters to grids and calls the solver. `web` must not contain physics.
- **Units.** SI (metres, volts) inside `solver`. Millimetres in `GunParameters` and the UI. Conversion happens in `wasm-api`. All fields carry unit suffixes (`_m`, `_mm`, `_v`, `_v_per_m`).
- **Coordinates.** z increases upward. Memory layout: `V[i_z * N_r + i_r]`, r is the fast axis. See PHYSICS.md §2.1–2.2.
- **r = 0 stencil.** The axis (i_r = 0) requires a different update rule than the interior (i_r ≥ 1). See PHYSICS.md §2.4. This must be a separate code path — applying the interior stencil at i_r = 0 is a silent correctness bug.
- **Error handling.** `Result` for runtime failures. Panics only for programmer errors (bugs in our own code, not bad user input).
- **Package manager.** Use `pnpm` (not `npm` or `yarn`) for all operations in `web/`.
