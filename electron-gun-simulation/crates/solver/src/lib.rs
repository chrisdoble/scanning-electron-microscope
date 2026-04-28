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
    #[error("outer boundary point (i_r={i_r}, i_z={i_z}) must be Fixed")]
    InvalidOuterBoundary { i_r: usize, i_z: usize },
    #[error(
        "solver did not converge after {iterations} iterations; final residual {residual_v} V"
    )]
    MaxIterationsExceeded { iterations: usize, residual_v: f64 },
}

/// Solves Laplace's equation in place on `potential`, updating only `Free`
/// points as marked in `mask`. Uses SOR as specified in PHYSICS.md §4.
///
/// The function validates the following before solving and returns the
/// corresponding `SolverError` if any check fails (without modifying the
/// potential grid):
/// - `potential` and `mask` have the same dimensions.
/// - The grid has at least 3 points in each direction.
/// - `config.omega` is in the range (0, 2).
/// - `config.tolerance_v` is strictly positive (> 0).
/// - All outer boundary points (i_r = N_r−1, i_z = 0, i_z = N_z−1) are
///   `Fixed` (any potential is allowed).
///
/// If the solver does not converge within `config.max_iterations`, it
/// returns `SolverError::MaxIterationsExceeded`.
///
/// The caller is responsible for:
/// - Pre-setting Dirichlet values on `Fixed` electrode points in the
///   potential grid.
/// - Leaving the axis (i_r = 0) as `Free` unless it lies on an electrode.
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

    let n_r = potential.n_r;
    let n_z = potential.n_z;

    // Validate outer boundary: every cell on i_z=0, i_z=n_z-1, and
    // i_r=n_r-1 must be Fixed (far-field Dirichlet, PHYSICS.md §2.5).
    for i_r in 0..n_r {
        for i_z in [0, n_z - 1] {
            let idx = potential.idx(i_r, i_z);
            if mask.data[idx] != Cell::Fixed {
                return Err(SolverError::InvalidOuterBoundary { i_r, i_z });
            }
        }
    }
    for i_z in 0..n_z {
        let idx = potential.idx(n_r - 1, i_z);
        if mask.data[idx] != Cell::Fixed {
            return Err(SolverError::InvalidOuterBoundary { i_r: n_r - 1, i_z });
        }
    }

    let omega = config.omega;

    // Retained across iterations so MaxIterationsExceeded can report it.
    let mut max_residual = 0.0_f64;

    for iteration in 0..config.max_iterations {
        max_residual = 0.0;

        // Row-major sweep: z is the slow axis, r the fast axis (PHYSICS.md §2.2).
        //
        // We only visit i_z in 1..n_z-1 and i_r in 0..n_r-1.  The outer
        // boundary rows/columns (i_z=0, i_z=n_z-1, i_r=n_r-1) have no valid
        // stencil and must be Fixed by the caller (ARCHITECTURE.md §3.3).
        for i_z in 1..n_z - 1 {
            // --- Axis (i_r = 0): equation (4), PHYSICS.md §2.4 ---
            {
                let idx = potential.idx(0, i_z);
                if mask.data[idx] == Cell::Free {
                    // V_GS = (1/6) [4 V[1,j] + V[0,j+1] + V[0,j−1]]
                    let v_gs = (4.0 * potential.data[potential.idx(1, i_z)]
                        + potential.data[potential.idx(0, i_z + 1)]
                        + potential.data[potential.idx(0, i_z - 1)])
                        / 6.0;
                    let v_old = potential.data[idx];
                    let v_new = (1.0 - omega) * v_old + omega * v_gs;
                    potential.data[idx] = v_new;
                    max_residual = max_residual.max((v_new - v_old).abs());
                }
            }

            // --- Interior (1 ≤ i_r ≤ N_r−2): equation (2), PHYSICS.md §2.3 ---
            for i_r in 1..n_r - 1 {
                let idx = potential.idx(i_r, i_z);
                if mask.data[idx] == Cell::Free {
                    let i = i_r as f64;
                    // V_GS = (1/4) [ V[i+1,j] (1+1/(2i))
                    //              + V[i−1,j] (1−1/(2i))
                    //              + V[i,j+1] + V[i,j−1] ]
                    let v_gs = (potential.data[potential.idx(i_r + 1, i_z)]
                        * (1.0 + 1.0 / (2.0 * i))
                        + potential.data[potential.idx(i_r - 1, i_z)] * (1.0 - 1.0 / (2.0 * i))
                        + potential.data[potential.idx(i_r, i_z + 1)]
                        + potential.data[potential.idx(i_r, i_z - 1)])
                        / 4.0;
                    let v_old = potential.data[idx];
                    let v_new = (1.0 - omega) * v_old + omega * v_gs;
                    potential.data[idx] = v_new;
                    max_residual = max_residual.max((v_new - v_old).abs());
                }
            }
        }

        if max_residual < config.tolerance_v {
            return Ok(SolverResult {
                iterations: iteration + 1,
                final_residual_v: max_residual,
            });
        }
    }

    Err(SolverError::MaxIterationsExceeded {
        iterations: config.max_iterations,
        residual_v: max_residual,
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
