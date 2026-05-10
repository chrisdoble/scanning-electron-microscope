# Architecture

This document specifies the code structure, crate layout, and interfaces between components. It is the authoritative reference for how the codebase is organised. `PHYSICS.md` specifies _what_ the solver computes; this document specifies _how_ the code is arranged and how components communicate.

## 1. Overview

The project contains a Cargo workspace with two Rust crates and a Vite-based TypeScript web application:

```
sem-gun-sim/
├── CLAUDE.md
├── docs/
│   ├── PHYSICS.md
│   ├── ARCHITECTURE.md         # this file
│   └── UI.md
├── crates/
│   ├── Cargo.toml              # workspace root
│   ├── solver/                 # pure Rust, generic Laplace solver
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   └── tests/
│   └── wasm-api/               # thin wasm-bindgen wrapper, gun-aware
│       ├── Cargo.toml
│       └── src/
└── web/                        # Vite + TypeScript application
    ├── package.json
    ├── index.html
    ├── src/
    └── public/
```

Three layers, each with a clear responsibility:

1. **`solver`** — pure Rust library. Knows about grids, stencils, sparse matrix assembly, and Cholesky factorisation. Knows nothing about electron guns. Native-testable with `cargo test`. No wasm dependencies.
2. **`wasm-api`** — thin Rust crate exposed to JavaScript via `wasm-bindgen`. Knows about `GunParameters`, electrode rasterisation, and the JS-facing API. Calls into `solver` for the potential solve and E-field extraction. Depends on `solver`.
3. **`web`** — Vite-based TypeScript application. Knows about UI state, sliders, visualisation, and the debounce/preview strategy. Consumes the wasm package.

Data flows one direction per user interaction: slider change in TS → `GunParameters` struct → wasm call → `GunSolution` back → render.

## 2. Rationale for the two-crate split

The `solver` and `wasm-api` are deliberately separate crates:

- **Native testability.** `solver` is a normal library crate (`crate-type = ["lib"]`) and runs under `cargo test` natively on any target. Tests run in milliseconds with a real debugger. `wasm-api` is a `cdylib` crate (required by `wasm-pack`), which cannot be depended on by other Rust crates — so solver logic placed there would not be importable for testing.
- **Dependency isolation.** `wasm-api` depends on `wasm-bindgen`, `js-sys`, etc., which only compile for `wasm32-unknown-unknown`. Keeping the solver free of these means it compiles cleanly for any target with no `#[cfg(target_arch = "wasm32")]` guards.
- **Layering.** Solver logic vs. JS-boundary logic are genuinely different concerns. A future CLI tool, native GUI, or Python binding via PyO3 would depend on `solver` and write a new thin wrapper — no untangling of wasm annotations from numerics.

## 3. The `solver` crate

### 3.1 Responsibility

Solves Laplace's equation on a 2D axisymmetric grid via sparse direct solve, given a grid of initial potential values and a mask marking which points are fixed (Dirichlet) versus free. Returns the solved potential grid. Also provides E-field extraction from a solved potential grid.

`solver` has no knowledge of electron guns, filaments, Wehnelts, or anodes. It sees only grids, masks, and numbers.

### 3.2 Dependencies

- **`faer`** — sparse Cholesky factorisation. This is the only significant dependency of the solver crate.

### 3.3 Core types

```rust
/// A 2D grid of f64 values stored in row-major order with z as the row index
/// and r as the column index. Layout matches PHYSICS.md §2.2.
pub struct Grid {
    pub n_r: usize,
    pub n_z: usize,
    pub h_m: f64,         // grid spacing in metres (dr = dz = h)
    pub data: Vec<f64>,   // length n_r * n_z, indexed as data[i_z * n_r + i_r]
                          // no unit suffix: may hold V, V/m, or other quantities
}

/// Per-point mask indicating whether the solver should update this point.
pub enum Cell {
    Free,
    Fixed,
}

pub struct Mask {
    pub n_r: usize,
    pub n_z: usize,
    pub data: Vec<Cell>, // same indexing as Grid
}
```

The `Grid` is used for both input (initial V with Dirichlet values set on `Fixed` points) and output (solved V). E-field grids (`E_r`, `E_z`) are also `Grid` values.

### 3.4 Core functions

```rust
pub struct SolverResult {
    pub n_free: usize,             // number of Free points solved for
}

/// Solves Laplace's equation in place on `potential`, computing the
/// values at Free points as marked in `mask`. Uses sparse Cholesky
/// factorisation as specified in PHYSICS.md §4.
///
/// The function validates the following before solving and returns the
/// corresponding SolverError if any check fails (without modifying the
/// potential grid):
/// - `potential` and `mask` have the same dimensions.
/// - The grid has at least 3 points in each direction.
///
/// The caller is responsible for:
/// - Pre-setting Dirichlet values on Fixed points in the potential grid.
/// - Marking the outer boundary (i_r = N_r-1, i_z = 0, i_z = N_z-1) as
///   Fixed (any potential is allowed). The axis (i_r = 0) should be Free
///   unless a point lies on an electrode.
pub fn solve_laplace_cylindrical(
    potential: &mut Grid,
    mask: &Mask,
) -> Result<SolverResult, SolverError>;

/// Computes E_r and E_z from a solved potential grid using the formulas
/// in PHYSICS.md §2.6. Returns (e_r, e_z) where both are Grid values with
/// the same dimensions as the input. Values are in V/m.
pub fn compute_electric_field(potential: &Grid) -> (Grid, Grid);
```

### 3.5 Error handling

```rust
pub enum SolverError {
    DimensionMismatch { grid: (usize, usize), mask: (usize, usize) },
    GridTooSmall { n_r: usize, n_z: usize },
    FactorisationFailed(String),
}
```

Use `thiserror` for the `Display` impl. No panics on runtime conditions — invalid input returns `Err`.

## 4. The `wasm-api` crate

### 4.1 Responsibility

Bridges `solver` to JavaScript. Translates a `GunParameters` struct from JS into the grid + mask representation the solver needs, invokes the solver, extracts E-fields, and packages the result for return to JS.

### 4.2 `GunParameters`

The parameters struct is the single source of truth for electron gun configuration. All dimensions are in **millimetres** (UI convention); the wasm-api crate converts to metres internally before calling the solver. All voltages are in volts, referenced to ground, unless otherwise stated.

```rust
#[wasm_bindgen]
#[derive(Clone, Copy)]
pub struct GunParameters {
    // --- Filament ---
    /// Radius of the filament, modelled as a solid disk centred on the
    /// z-axis. Because the simulation is axisymmetric, only the r ≥ 0
    /// half of the disk appears in the grid; the r < 0 half is implied
    /// by symmetry. In the (r, z) cross-section, the filament is a
    /// rectangle from r=0 to r=filament_radius, and from
    /// z=filament_z − thickness/2 to z=filament_z + thickness/2.
    pub filament_radius_mm: f64,
    /// Full thickness of the filament in z.
    pub filament_thickness_mm: f64,
    /// z-coordinate of the filament centre.
    pub filament_z_mm: f64,
    /// Voltage applied to the filament, referenced to ground
    /// (typically a large negative value, e.g. -10_000 V).
    pub filament_voltage_v: f64,

    // --- Wehnelt cylinder ---
    /// Outer radius of the cylindrical wall.
    pub wehnelt_outer_radius_mm: f64,
    /// Inner radius of the cylindrical wall (gives wall thickness).
    pub wehnelt_inner_radius_mm: f64,
    /// z-coordinate of the centre of the cylindrical wall.
    pub wehnelt_z_mm: f64,
    /// Total height of the cylindrical wall. The wall runs from
    /// z − height/2 to z + height/2. The open end is at the top
    /// (z + height/2); the cap is at the bottom (see below).
    pub wehnelt_height_mm: f64,
    /// Full thickness of the front cap plate. The cap sits at the
    /// bottom of the cylinder, from z − height/2 − cap_thickness
    /// to z − height/2. The aperture is in this cap.
    pub wehnelt_cap_thickness_mm: f64,
    /// Radius of the aperture in the front cap.
    pub wehnelt_aperture_radius_mm: f64,
    /// Voltage bias of the Wehnelt relative to the filament (typically
    /// a few hundred volts more negative, e.g. -200 V). The Wehnelt's
    /// absolute voltage is filament_voltage_v + wehnelt_bias_v.
    pub wehnelt_bias_v: f64,

    // --- Anode ---
    /// z-coordinate of the centre of the anode plate.
    pub anode_z_mm: f64,
    /// Full thickness of the anode plate in z.
    pub anode_thickness_mm: f64,
    /// Outer radius of the anode plate.
    pub anode_outer_radius_mm: f64,
    /// Radius of the aperture in the anode.
    pub anode_aperture_radius_mm: f64,
    /// Voltage applied to the anode, referenced to ground
    /// (typically 0 V / ground; defaults to 0 V).
    pub anode_voltage_v: f64,
}
```

Conventions:

- z increases upward. The Wehnelt's open end is always at the top (higher z) and its cap with aperture is at the bottom (lower z). The filament enters from the top and sits inside the cup. The beam exits downward through the cap aperture toward the anode. Typical z ordering: `anode_z_mm < wehnelt_z_mm − wehnelt_height_mm/2 < wehnelt_z_mm < filament_z_mm`.
- All electrodes use centre z-position + full thickness/height. Rasterisation computes the z-extent as ±thickness/2 (or ±height/2) from the centre. The Wehnelt cap is the one exception: it sits below the cylinder, from `wehnelt_z_mm − wehnelt_height_mm/2 − wehnelt_cap_thickness_mm` to `wehnelt_z_mm − wehnelt_height_mm/2`.
- Geometric consistency is validated in `wasm-api` before rasterisation. Validation failures are returned as errors through the API.

### 4.3 Rasterisation

Rasterisation converts `GunParameters` into a `Grid` (with Dirichlet voltages set) and a `Mask` (marking Fixed vs Free points). The process is:

1. Convert all dimensions from mm to metres.
2. Determine the simulation domain: compute the bounding box of all electrodes, then extend by 3× the largest electrode dimension in each direction (per PHYSICS.md §3.2). This gives R_max and Z_max.
3. Choose h = min(smallest electrode dimension) / 10 per PHYSICS.md §2.1. Compute N_r and N_z from R_max, Z_max, and h.
4. Allocate a `Grid` (all zeros) and `Mask` (all `Free`).
5. Mark the outer boundary as `Fixed`: all points where i_r = N_r−1, i_z = 0, or i_z = N_z−1 are set to `Fixed` in the mask and 0.0 in the potential grid. (Points at i_r = 0 are left `Free` — they are axis points handled by the r=0 stencil, unless they happen to lie on an electrode.)
6. For each grid point, test whether its physical location (r, z) lies inside any electrode's (r, z) cross-section. Each electrode's cross-section is one or more axis-aligned rectangles:
   - **Filament:** a single rectangle. Point (r, z) is inside if `r ≤ filament_radius` AND `|z − filament_z| ≤ filament_thickness / 2`.
   - **Wehnelt wall:** a single rectangle. Point (r, z) is inside if `wehnelt_inner_radius ≤ r ≤ wehnelt_outer_radius` AND `wehnelt_z − wehnelt_height/2 ≤ z ≤ wehnelt_z + wehnelt_height/2`.
   - **Wehnelt cap:** a single rectangle (with the aperture excluded). Point (r, z) is inside if `wehnelt_aperture_radius ≤ r ≤ wehnelt_outer_radius` AND `wehnelt_z − wehnelt_height/2 − wehnelt_cap_thickness ≤ z ≤ wehnelt_z − wehnelt_height/2`.
   - **Anode:** a single rectangle (with the aperture excluded). Point (r, z) is inside if `anode_aperture_radius ≤ r ≤ anode_outer_radius` AND `|z − anode_z| ≤ anode_thickness / 2`.
7. If a point lies inside an electrode, set the mask to `Fixed` and set the potential grid value to the electrode's voltage. The Wehnelt's absolute voltage is `filament_voltage_v + wehnelt_bias_v`. If a point lies inside multiple electrodes (which should not happen for valid geometry), the last one checked wins.

All dimension comparisons use physical coordinates in metres.

### 4.4 `GunSolution`

The solution struct returned to JS. Contains the potential and field grids plus metadata. The solver-internal `Grid` and `Mask` types are not `#[wasm_bindgen]`-compatible, so this struct unpacks them into flat `Vec`s exposed to JS.

The `#[wasm_bindgen(getter_with_clone)]` attribute on the grid fields tells wasm-bindgen to auto-generate cloning getters for `Vec` types that can't be simply copied across the boundary. `Vec<f64>` fields become JS `Float64Array` properties, and `Vec<u8>` becomes `Uint8Array`. The `#[wasm_bindgen(readonly)]` attribute on each field prevents JS from setting them.

```rust
#[wasm_bindgen]
pub struct GunSolution {
    #[wasm_bindgen(readonly)]
    pub n_r: usize,
    #[wasm_bindgen(readonly)]
    pub n_z: usize,
    #[wasm_bindgen(readonly)]
    pub h_m: f64,                // grid spacing in metres

    // All grids are length n_r * n_z, row-major as per PHYSICS.md §2.2.
    // Accessing these from JS returns a copy (clone) of the data.
    #[wasm_bindgen(getter_with_clone, readonly)]
    pub potential_v: Vec<f64>,   // potential in volts
    #[wasm_bindgen(getter_with_clone, readonly)]
    pub e_r_v_per_m: Vec<f64>,  // radial E-field component in V/m
    #[wasm_bindgen(getter_with_clone, readonly)]
    pub e_z_v_per_m: Vec<f64>,  // axial E-field component in V/m
    #[wasm_bindgen(getter_with_clone, readonly)]
    pub mask: Vec<u8>,           // 0 = Free, 1 = Fixed; useful for UI overlays
}
```

Each access to a `Vec` field from JS (e.g. `solution.potential_v`) clones the data from wasm linear memory into a new JS-owned typed array. For our grid sizes (a few hundred thousand floats, ~1-2 MB) this takes microseconds. The TS code should cache the result rather than accessing the property repeatedly.

### 4.5 Entry point

```rust
/// Solve the electron gun potential field.
#[wasm_bindgen]
pub fn solve_electron_gun(
    params: &GunParameters,
) -> Result<GunSolution, JsError>;
```

Internal steps:

1. Validate `params` (geometric consistency).
2. Determine domain size and grid spacing (see §4.3 steps 1-3).
3. Rasterise electrodes into grid and mask (see §4.3 steps 4-7).
4. Call `solver::solve_laplace_cylindrical`.
5. Call `solver::compute_electric_field`.
6. Package into `GunSolution` and return.

### 4.6 Error handling across the wasm boundary

`wasm-bindgen` maps `Result<T, JsError>` to JS exceptions. `JsError` is imported from `wasm-bindgen` (`use wasm_bindgen::JsError;`). On the JS side, an `Err` becomes a thrown exception that the calling code catches with try/catch. The wasm-api uses:

```rust
pub fn solve_electron_gun(...) -> Result<GunSolution, JsError>
```

Internal errors (geometry validation failures, solver failures) are converted to `JsError` with descriptive messages that the TS side can catch and display.

## 5. The `web` TypeScript application

### 5.1 Responsibility

UI and visualisation. Owns the parameter state (what the sliders control), calls into wasm when parameters change, and renders the result.

Does not contain any physics or numerical logic.

### 5.2 Build setup

- Vite as the dev server and build tool.
- TypeScript with `strict` enabled.
- The wasm package is imported from `../crates/wasm-api/pkg` (generated by `wasm-pack build --target web`).
- Development loop: a `cargo watch` process in a second terminal rebuilds the wasm package on Rust changes; Vite's HMR picks up the new package automatically.

### 5.3 Rough structure

Detailed UI architecture (components, state management, visualisation, debounce strategy) lives in `UI.md`. This document only specifies the boundary.

## 6. Build and development

### 6.1 Toolchain

- Rust stable (most recent release).
- `wasm-pack` installed via `cargo install wasm-pack`.
- `wasm32-unknown-unknown` target installed via `rustup target add wasm32-unknown-unknown`.
- Node.js (LTS) for the web app.

### 6.2 Commands

Run from the project root unless stated otherwise:

```bash
# Native tests for the solver (fast, primary development loop for physics)
cargo test -p solver --manifest-path crates/Cargo.toml

# Build the wasm package (outputs to crates/wasm-api/pkg/)
wasm-pack build --target web crates/wasm-api

# Watch + rebuild on changes
cargo watch -s 'wasm-pack build --target web crates/wasm-api'

# Run the web dev server
cd web && pnpm run dev

# Production build of the web app
cd web && pnpm run build
```

### 6.3 Workspace configuration

```toml
# crates/Cargo.toml (workspace root)
[workspace]
members = ["solver", "wasm-api"]
resolver = "2"
```

`solver` has `crate-type = ["lib"]` (default). `wasm-api` has `crate-type = ["cdylib"]`. Tests for `wasm-api` use `wasm-bindgen-test` and run in a headless browser or Node; these are lower priority since the layer is thin.

## 7. Conventions

- **Units.** SI (metres, volts, seconds) inside the solver, with unit suffixes on field names (`_m`, `_v`, `_v_per_m`). Millimetres at the UI layer and in `GunParameters` (`_mm`). Conversion happens in `wasm-api`. Fields without a unit suffix (like `Grid.data`) are generic and may hold values in different units depending on context.
- **Coordinates.** z increases upward (PHYSICS.md §2.1). Memory layout has r as the fast axis (PHYSICS.md §2.2).
- **Error handling.** `Result` for anything that can fail at runtime. Panics are reserved for programmer errors (contract violations that would indicate a bug in our own code, not bad user input).
- **Testing.** Physics correctness is tested natively in `crates/solver/tests/`. The wasm-api layer is tested via `wasm-bindgen-test`; these are lower priority since the layer is thin. The TypeScript side is tested with Vitest for any non-trivial logic.
- **No cross-layer leakage.** `solver` must not know about guns; `wasm-api` must not contain numerics; `web` must not contain physics. If a change seems to require breaking one of these rules, that is a signal to rethink — not to break the rule.
