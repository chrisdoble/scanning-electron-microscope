use solver::{compute_electric_field, solve_laplace_cylindrical, Cell, Grid, Mask};
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

    /// Multiplier applied to the computed grid spacing h. Values > 1.0
    /// produce a coarser grid that solves faster (useful for live preview
    /// while dragging). Default 1.0 = full resolution.
    pub h_scale: f64,
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
            h_scale: 1.0,
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
pub fn solve_electron_gun(params: &GunParameters) -> Result<GunSolution, JsError> {
    // 1. Convert mm → m (ARCHITECTURE.md §4.2 convention).
    let fil_r = params.filament_radius_mm * 1e-3;
    let fil_t = params.filament_thickness_mm * 1e-3;
    let fil_z = params.filament_z_mm * 1e-3;
    let fil_v = params.filament_voltage_v;

    let weh_outer_r = params.wehnelt_outer_radius_mm * 1e-3;
    let weh_inner_r = params.wehnelt_inner_radius_mm * 1e-3;
    let weh_z = params.wehnelt_z_mm * 1e-3;
    let weh_height = params.wehnelt_height_mm * 1e-3;
    let weh_cap_t = params.wehnelt_cap_thickness_mm * 1e-3;
    let weh_ap_r = params.wehnelt_aperture_radius_mm * 1e-3;
    let weh_v = params.filament_voltage_v + params.wehnelt_bias_v;

    let an_z = params.anode_z_mm * 1e-3;
    let an_t = params.anode_thickness_mm * 1e-3;
    let an_outer_r = params.anode_outer_radius_mm * 1e-3;
    let an_ap_r = params.anode_aperture_radius_mm * 1e-3;
    let an_v = params.anode_voltage_v;

    // 2. Validate geometry (ARCHITECTURE.md §4.3).
    if weh_inner_r >= weh_outer_r {
        return Err(JsError::new(
            "Wehnelt inner radius must be less than outer radius",
        ));
    }
    if weh_ap_r >= weh_outer_r {
        return Err(JsError::new(
            "Wehnelt aperture radius must be less than outer radius",
        ));
    }
    if an_ap_r >= an_outer_r {
        return Err(JsError::new(
            "Anode aperture radius must be less than outer radius",
        ));
    }
    if fil_r >= weh_inner_r {
        return Err(JsError::new(
            "Filament radius must be less than Wehnelt inner radius",
        ));
    }

    // 3. Electrode z-extents.
    let fil_z_lo = fil_z - fil_t / 2.0;
    let fil_z_hi = fil_z + fil_t / 2.0;
    let weh_wall_z_lo = weh_z - weh_height / 2.0;
    let weh_wall_z_hi = weh_z + weh_height / 2.0;
    let weh_cap_z_lo = weh_wall_z_lo - weh_cap_t;
    let weh_cap_z_hi = weh_wall_z_lo;
    let an_z_lo = an_z - an_t / 2.0;
    let an_z_hi = an_z + an_t / 2.0;

    // 4. Bounding box of all electrodes.
    let elec_r_max = weh_outer_r.max(an_outer_r);
    let elec_z_min = fil_z_lo.min(weh_wall_z_lo).min(weh_cap_z_lo).min(an_z_lo);
    let elec_z_max = fil_z_hi.max(weh_wall_z_hi).max(an_z_hi);

    // Largest single-electrode dimension sets the far-field margin
    // (ARCHITECTURE.md §4.3 step 2, PHYSICS.md §3.2).
    let largest_dim = [
        fil_r,
        fil_t,
        weh_outer_r,
        weh_height + weh_cap_t,
        an_outer_r,
        an_t,
    ]
    .into_iter()
    .fold(0.0_f64, f64::max);

    let margin = 3.0 * largest_dim;
    let r_max = elec_r_max + margin;
    let z_lo = elec_z_min - margin; // physical z at i_z = 0
    let z_range = (elec_z_max + margin) - z_lo;

    // 5. Grid spacing h = min(smallest electrode dimension) / 10
    //    (PHYSICS.md §2.1).  Floored at 50 µm so the default filament
    //    thickness (50 µm) always gets at least one rasterised cell.
    let smallest_dim = [
        fil_r,
        fil_t,
        weh_outer_r - weh_inner_r,
        weh_cap_t,
        weh_ap_r,
        weh_outer_r - weh_ap_r,
        an_outer_r - an_ap_r,
        an_t,
        an_ap_r,
    ]
    .into_iter()
    .filter(|&x| x > 0.0)
    .fold(f64::INFINITY, f64::min);

    let h = (smallest_dim / 10.0).max(5e-5) * params.h_scale;

    let n_r = (r_max / h).ceil() as usize + 1;
    let n_z = (z_range / h).ceil() as usize + 1;

    // 6. Allocate grid and mask; fix outer boundary at V = 0 (ARCHITECTURE.md §4.3 step 5).
    let mut potential = Grid::new(n_r, n_z, h);
    let mut mask = Mask::new(n_r, n_z);

    for i_r in 0..n_r {
        mask.data[potential.idx(i_r, 0)] = Cell::Fixed; // z = z_lo (bottom)
        mask.data[potential.idx(i_r, n_z - 1)] = Cell::Fixed; // z = z_hi (top)
    }
    for i_z in 0..n_z {
        mask.data[potential.idx(n_r - 1, i_z)] = Cell::Fixed; // r = r_max (outer)
    }

    // 7. Rasterise electrodes (ARCHITECTURE.md §4.3 steps 6-7).
    //    Physical coords: r = i_r * h, z = z_lo + i_z * h.
    //    Later electrodes overwrite earlier ones (last-wins).
    for i_z in 0..n_z {
        let z = z_lo + i_z as f64 * h;
        for i_r in 0..n_r {
            let r = i_r as f64 * h;
            let idx = potential.idx(i_r, i_z);

            // Filament: solid disk, r ≤ fil_r, |z − fil_z| ≤ fil_t/2.
            if r <= fil_r && z >= fil_z_lo && z <= fil_z_hi {
                mask.data[idx] = Cell::Fixed;
                potential.data[idx] = fil_v;
            }
            // Wehnelt wall: weh_inner_r ≤ r ≤ weh_outer_r, wall z-range.
            if r >= weh_inner_r && r <= weh_outer_r && z >= weh_wall_z_lo && z <= weh_wall_z_hi {
                mask.data[idx] = Cell::Fixed;
                potential.data[idx] = weh_v;
            }
            // Wehnelt cap (aperture excluded): weh_ap_r ≤ r ≤ weh_outer_r, cap z-range.
            if r >= weh_ap_r && r <= weh_outer_r && z >= weh_cap_z_lo && z <= weh_cap_z_hi {
                mask.data[idx] = Cell::Fixed;
                potential.data[idx] = weh_v;
            }
            // Anode (aperture excluded): an_ap_r ≤ r ≤ an_outer_r, |z − an_z| ≤ an_t/2.
            if r >= an_ap_r && r <= an_outer_r && z >= an_z_lo && z <= an_z_hi {
                mask.data[idx] = Cell::Fixed;
                potential.data[idx] = an_v;
            }
        }
    }

    // 8. Solve.
    solve_laplace_cylindrical(&mut potential, &mask).map_err(|e| JsError::new(&e.to_string()))?;

    let (e_r, e_z) = compute_electric_field(&potential);

    let mask_u8: Vec<u8> = mask
        .data
        .iter()
        .map(|c| if *c == Cell::Free { 0u8 } else { 1u8 })
        .collect();

    Ok(GunSolution {
        n_r: potential.n_r,
        n_z: potential.n_z,
        h_m: potential.h_m,
        potential_v: potential.data,
        e_r_v_per_m: e_r.data,
        e_z_v_per_m: e_z.data,
        mask: mask_u8,
    })
}
