use solver::{compute_electric_field, solve_laplace_cylindrical, Cell, Grid, Mask, SolverConfig};
use wasm_bindgen::prelude::*;

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

impl Default for GunParameters {
    fn default() -> Self {
        Self {
            filament_radius_mm: 0.15,
            filament_thickness_mm: 0.05,
            filament_z_mm: 10.0,
            filament_voltage_v: -10_000.0,
            wehnelt_outer_radius_mm: 3.0,
            wehnelt_inner_radius_mm: 2.5,
            wehnelt_z_mm: 8.0,
            wehnelt_height_mm: 6.0,
            wehnelt_cap_thickness_mm: 0.5,
            wehnelt_aperture_radius_mm: 0.5,
            wehnelt_bias_v: -200.0,
            anode_z_mm: 3.0,
            anode_thickness_mm: 1.0,
            anode_outer_radius_mm: 3.0,
            anode_aperture_radius_mm: 0.5,
            anode_voltage_v: 0.0,
        }
    }
}

#[wasm_bindgen]
impl GunParameters {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }
}

#[wasm_bindgen]
pub struct GunSolution {
    #[wasm_bindgen(readonly)]
    pub n_r: usize,
    #[wasm_bindgen(readonly)]
    pub n_z: usize,
    #[wasm_bindgen(readonly)]
    pub h_m: f64, // grid spacing in metres
    #[wasm_bindgen(readonly)]
    pub iterations: u32,

    // All grids are length n_r * n_z, row-major as per PHYSICS.md §2.2.
    // Accessing these from JS returns a copy (clone) of the data.
    potential_v: Vec<f64>, // potential in volts
    e_r_v_per_m: Vec<f64>, // radial E-field component in V/m
    e_z_v_per_m: Vec<f64>, // axial E-field component in V/m
    mask: Vec<u8>,         // 0 = Free, 1 = Fixed; useful for UI overlays
}

// By default, wasm-bindgen expresses Vec<f64> as Float64Array and Vec<u8> as
// Uint8Array in TypeScript. Those types are generic on the type of ArrayBuffer
// backing them (ArrayBuffer or SharedArrayBuffer). The latter is only used
// when multithreading is enabled (which we're not using) so the types are
// unnecessarily generic which causes errors with (de)serialisation. Define
// getters here with custom return types to work around this.
#[wasm_bindgen]
impl GunSolution {
    #[wasm_bindgen(getter, unchecked_return_type = "Float64Array<ArrayBuffer>")]
    pub fn potential_v(&self) -> Vec<f64> {
        self.potential_v.clone()
    }

    #[wasm_bindgen(getter, unchecked_return_type = "Float64Array<ArrayBuffer>")]
    pub fn e_r_v_per_m(&self) -> Vec<f64> {
        self.e_r_v_per_m.clone()
    }

    #[wasm_bindgen(getter, unchecked_return_type = "Float64Array<ArrayBuffer>")]
    pub fn e_z_v_per_m(&self) -> Vec<f64> {
        self.e_z_v_per_m.clone()
    }

    #[wasm_bindgen(getter, unchecked_return_type = "Uint8Array<ArrayBuffer>")]
    pub fn mask(&self) -> Vec<u8> {
        self.mask.clone()
    }
}

/// Solve the electron gun potential field.
#[wasm_bindgen]
pub fn solve_electron_gun(_params: &GunParameters) -> Result<GunSolution, JsError> {
    // TODO: rasterise geometry from _params (ARCHITECTURE.md §4.3).
    //
    // Stub geometry: a horizontal band fixed at -1 000 V, running from the
    // axis (i_r = 0) to i_r = 24 (just past half the grid width) at rows
    // i_z = 18–21.  This exercises the mirrored cross-section rendering
    // without needing a real gun geometry.
    let n_r: usize = 50;
    let n_z: usize = 40;
    let mut potential = Grid::new(n_r, n_z, 1e-3);
    let mut mask = Mask::new(n_r, n_z);

    for i_z in 18..22_usize {
        for i_r in 0..25_usize {
            let idx = mask.idx(i_r, i_z);
            mask.data[idx] = Cell::Fixed;
            potential.data[idx] = -1_000.0;
        }
    }

    let result = solve_laplace_cylindrical(&mut potential, &mask, &SolverConfig::default())
        .map_err(|e| JsError::new(&e.to_string()))?;

    let (e_r, e_z) = compute_electric_field(&potential);

    let mask_u8 = mask
        .data
        .iter()
        .map(|c| if *c == Cell::Free { 0u8 } else { 1u8 })
        .collect();

    Ok(GunSolution {
        n_r: potential.n_r,
        n_z: potential.n_z,
        h_m: potential.h_m,
        iterations: result.iterations as u32,
        potential_v: potential.data,
        e_r_v_per_m: e_r.data,
        e_z_v_per_m: e_z.data,
        mask: mask_u8,
    })
}
