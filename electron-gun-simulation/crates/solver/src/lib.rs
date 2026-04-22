use thiserror::Error;

/// 2D grid of f64 values, row-major with z as the row index and r as the
/// column index. Index formula: data[i_z * n_r + i_r]. See PHYSICS.md §2.2.
pub struct Grid {
    pub n_r: usize,
    pub n_z: usize,
    /// Grid spacing in metres; dr = dz = h (see PHYSICS.md §2.1).
    pub h_m: f64,
    pub data: Vec<f64>,
}

impl Grid {
    pub fn new(n_r: usize, n_z: usize, h_m: f64) -> Self {
        Self {
            n_r,
            n_z,
            h_m,
            data: vec![0.0; n_r * n_z],
        }
    }

    #[inline]
    pub fn idx(&self, i_r: usize, i_z: usize) -> usize {
        i_z * self.n_r + i_r
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    Free,
    Fixed,
}

pub struct Mask {
    pub n_r: usize,
    pub n_z: usize,
    pub data: Vec<Cell>,
}

impl Mask {
    pub fn new(n_r: usize, n_z: usize) -> Self {
        Self {
            n_r,
            n_z,
            data: vec![Cell::Free; n_r * n_z],
        }
    }

    #[inline]
    pub fn idx(&self, i_r: usize, i_z: usize) -> usize {
        i_z * self.n_r + i_r
    }
}

pub struct SolverConfig {
    /// SOR relaxation factor; must be in (0, 2).
    pub omega: f64,
    /// Absolute convergence threshold in volts; must be > 0.
    /// Caller should set this to 1e-6 × V_max (see PHYSICS.md §4.2).
    pub tolerance_v: f64,
    /// Hard iteration cap.
    pub max_iterations: usize,
}

impl Default for SolverConfig {
    fn default() -> Self {
        Self {
            omega: 1.9,
            tolerance_v: 1e-6,
            max_iterations: 50_000,
        }
    }
}

pub struct SolverResult {
    pub iterations: usize,
    pub final_residual_v: f64,
}

#[derive(Debug, Error)]
pub enum SolverError {
    #[error("grid size {grid:?} does not match mask size {mask:?}")]
    DimensionMismatch {
        grid: (usize, usize),
        mask: (usize, usize),
    },
    #[error("grid too small (n_r={n_r}, n_z={n_z}); minimum is 3 in each direction")]
    GridTooSmall { n_r: usize, n_z: usize },
    #[error("omega {0} is not in (0, 2)")]
    InvalidOmega(f64),
    #[error("tolerance {0} is not strictly positive")]
    InvalidTolerance(f64),
    #[error("solver did not converge after {iterations} iterations; final residual {residual_v} V")]
    MaxIterationsExceeded { iterations: usize, residual_v: f64 },
}

/// Solves Laplace's equation in place on `potential`, updating only `Free`
/// points. See PHYSICS.md §4 and ARCHITECTURE.md §3.3.
pub fn solve_laplace_cylindrical(
    potential: &mut Grid,
    mask: &Mask,
    config: &SolverConfig,
) -> Result<SolverResult, SolverError> {
    if potential.n_r != mask.n_r || potential.n_z != mask.n_z {
        return Err(SolverError::DimensionMismatch {
            grid: (potential.n_r, potential.n_z),
            mask: (mask.n_r, mask.n_z),
        });
    }
    if potential.n_r < 3 || potential.n_z < 3 {
        return Err(SolverError::GridTooSmall {
            n_r: potential.n_r,
            n_z: potential.n_z,
        });
    }
    if config.omega <= 0.0 || config.omega >= 2.0 {
        return Err(SolverError::InvalidOmega(config.omega));
    }
    if config.tolerance_v <= 0.0 {
        return Err(SolverError::InvalidTolerance(config.tolerance_v));
    }

    // TODO: implement SOR iteration (PHYSICS.md §4).
    Ok(SolverResult {
        iterations: 0,
        final_residual_v: 0.0,
    })
}

/// Computes (E_r, E_z) from a converged potential grid using finite
/// differences. See PHYSICS.md §2.6. Values are in V/m.
pub fn compute_electric_field(potential: &Grid) -> (Grid, Grid) {
    // TODO: implement finite-difference E-field extraction (PHYSICS.md §2.6).
    let e_r = Grid::new(potential.n_r, potential.n_z, potential.h_m);
    let e_z = Grid::new(potential.n_r, potential.n_z, potential.h_m);
    (e_r, e_z)
}
