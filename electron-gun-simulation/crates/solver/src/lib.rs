use faer::prelude::*;
use faer::sparse::{SparseColMat, Triplet};
use faer::{Mat, Side};
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

pub struct SolverResult {
    pub n_free: usize,
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
    #[error("outer boundary point (i_r={i_r}, i_z={i_z}) must be Fixed")]
    InvalidOuterBoundary { i_r: usize, i_z: usize },
    #[error("solver failed: {0}")]
    FactorisationFailed(String),
}

/// Solves Laplace's equation in place on `potential`, updating only `Free`
/// points as marked in `mask`. Uses sparse Cholesky factorisation as
/// specified in PHYSICS.md §4.
///
/// The function validates the following before solving and returns the
/// corresponding `SolverError` if any check fails (without modifying the
/// potential grid):
/// - `potential` and `mask` have the same dimensions.
/// - The grid has at least 3 points in each direction.
/// - All outer boundary points (i_r = N_r−1, i_z = 0, i_z = N_z−1) are
///   `Fixed`.
///
/// The caller is responsible for:
/// - Pre-setting Dirichlet values on `Fixed` electrode points in the
///   potential grid.
/// - Leaving the axis (i_r = 0) as `Free` unless it lies on an electrode.
pub fn solve_laplace_cylindrical(
    potential: &mut Grid,
    mask: &Mask,
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

    // --- Step 1: Build free-point index map ---
    //
    // free_idx[flat_grid_index] = Some(k) for Free points, None for Fixed.
    // free_points[k] = flat_grid_index for the k-th equation.
    let mut free_idx: Vec<Option<usize>> = vec![None; n_r * n_z];
    let mut free_points: Vec<usize> = Vec::new();
    for i_z in 0..n_z {
        for i_r in 0..n_r {
            let flat = potential.idx(i_r, i_z);
            if mask.data[flat] == Cell::Free {
                free_idx[flat] = Some(free_points.len());
                free_points.push(flat);
            }
        }
    }
    let n_free = free_points.len();
    if n_free == 0 {
        return Ok(SolverResult { n_free: 0 });
    }

    // --- Step 2: Assemble COO triplets and RHS ---
    //
    // Uses the symmetry-weighted stencils from PHYSICS.md §4.5. Fixed
    // neighbours move their contribution to b (the RHS), while Free
    // neighbours add off-diagonal entries to the matrix.
    let mut triplets: Vec<Triplet<usize, usize, f64>> = Vec::with_capacity(5 * n_free);
    let mut b: Vec<f64> = vec![0.0; n_free];

    for (k, &flat) in free_points.iter().enumerate() {
        let i_r = flat % n_r;
        let i_z = flat / n_r;

        if i_r == 0 {
            // Axis stencil: weighted equation (6), PHYSICS.md §4.5.
            //   self: −6/8,  r+1: 1/2,  z+1: 1/8,  z−1: 1/8
            triplets.push(Triplet::new(k, k, -6.0 / 8.0));

            let flat_r1 = potential.idx(1, i_z);
            match free_idx[flat_r1] {
                Some(k2) => triplets.push(Triplet::new(k, k2, 0.5)),
                None => b[k] -= 0.5 * potential.data[flat_r1],
            }

            let flat_zp = potential.idx(0, i_z + 1);
            match free_idx[flat_zp] {
                Some(k2) => triplets.push(Triplet::new(k, k2, 1.0 / 8.0)),
                None => b[k] -= potential.data[flat_zp] / 8.0,
            }

            let flat_zm = potential.idx(0, i_z - 1);
            match free_idx[flat_zm] {
                Some(k2) => triplets.push(Triplet::new(k, k2, 1.0 / 8.0)),
                None => b[k] -= potential.data[flat_zm] / 8.0,
            }
        } else {
            // Interior stencil: weighted equation (5), PHYSICS.md §4.5.
            //   self: −4i,  r+1: i+½,  r−1: i−½,  z+1: i,  z−1: i
            let i = i_r as f64;
            triplets.push(Triplet::new(k, k, -4.0 * i));

            let flat_rp = potential.idx(i_r + 1, i_z);
            let c_rp = i + 0.5;
            match free_idx[flat_rp] {
                Some(k2) => triplets.push(Triplet::new(k, k2, c_rp)),
                None => b[k] -= c_rp * potential.data[flat_rp],
            }

            let flat_rm = potential.idx(i_r - 1, i_z);
            let c_rm = i - 0.5;
            match free_idx[flat_rm] {
                Some(k2) => triplets.push(Triplet::new(k, k2, c_rm)),
                None => b[k] -= c_rm * potential.data[flat_rm],
            }

            let flat_zp = potential.idx(i_r, i_z + 1);
            match free_idx[flat_zp] {
                Some(k2) => triplets.push(Triplet::new(k, k2, i)),
                None => b[k] -= i * potential.data[flat_zp],
            }

            let flat_zm = potential.idx(i_r, i_z - 1);
            match free_idx[flat_zm] {
                Some(k2) => triplets.push(Triplet::new(k, k2, i)),
                None => b[k] -= i * potential.data[flat_zm],
            }
        }
    }

    // --- Step 3: Negate to obtain an SPD system ---
    //
    // A as assembled has a negative diagonal, so −A is positive definite.
    // Solve (−A)x = −b, which has the same solution x as Ax = b.
    for t in &mut triplets {
        t.val = -t.val;
    }
    for bi in &mut b {
        *bi = -*bi;
    }

    // --- Step 4: Assemble CSC matrix, factorise, solve ---
    let mat =
        SparseColMat::<usize, f64>::try_new_from_triplets(n_free, n_free, &triplets)
            .map_err(|e| SolverError::FactorisationFailed(format!("matrix assembly: {e}")))?;

    let llt = mat
        .sp_cholesky(Side::Lower)
        .map_err(|e| SolverError::FactorisationFailed(format!("Cholesky: {e:?}")))?;

    let rhs = Mat::<f64>::from_fn(n_free, 1, |i, _| b[i]);
    let x = llt.solve(rhs);

    // --- Step 5: Write solution back to the potential grid ---
    for (k, v) in x.rb().col(0).iter().enumerate() {
        potential.data[free_points[k]] = *v;
    }

    Ok(SolverResult { n_free })
}

/// Computes (E_r, E_z) from a converged potential grid using finite
/// differences. See PHYSICS.md §2.6. Values are in V/m.
pub fn compute_electric_field(potential: &Grid) -> (Grid, Grid) {
    let n_r = potential.n_r;
    let n_z = potential.n_z;
    let h = potential.h_m;

    let mut e_r = Grid::new(n_r, n_z, h);
    let mut e_z = Grid::new(n_r, n_z, h);

    // Shorthand helpers.
    let v = |i_r: usize, i_z: usize| potential.data[potential.idx(i_r, i_z)];
    let set_e = |e: &mut Grid, i_r: usize, i_z: usize, val: f64| {
        let k = e.idx(i_r, i_z);
        e.data[k] = val;
    };

    // Interior: 1 ≤ i_r ≤ N_r−2, 1 ≤ i_z ≤ N_z−2 — central differences.
    for i_z in 1..n_z - 1 {
        for i_r in 1..n_r - 1 {
            set_e(&mut e_r, i_r, i_z, -(v(i_r + 1, i_z) - v(i_r - 1, i_z)) / (2.0 * h));
            set_e(&mut e_z, i_r, i_z, -(v(i_r, i_z + 1) - v(i_r, i_z - 1)) / (2.0 * h));
        }
    }

    // Axis: i_r = 0, 1 ≤ i_z ≤ N_z−2.
    // E_r[0, j] = 0 by symmetry (already zero from Grid::new).
    for i_z in 1..n_z - 1 {
        set_e(&mut e_z, 0, i_z, -(v(0, i_z + 1) - v(0, i_z - 1)) / (2.0 * h));
    }

    // Outer radial edge: i_r = N_r−1, 1 ≤ i_z ≤ N_z−2.
    // E_r: backward one-sided; E_z: central.
    for i_z in 1..n_z - 1 {
        set_e(&mut e_r, n_r - 1, i_z, -(v(n_r - 1, i_z) - v(n_r - 2, i_z)) / h);
        set_e(&mut e_z, n_r - 1, i_z, -(v(n_r - 1, i_z + 1) - v(n_r - 1, i_z - 1)) / (2.0 * h));
    }

    // Bottom edge: 1 ≤ i_r ≤ N_r−2, i_z = 0.
    // E_r: central; E_z: forward one-sided.
    for i_r in 1..n_r - 1 {
        set_e(&mut e_r, i_r, 0, -(v(i_r + 1, 0) - v(i_r - 1, 0)) / (2.0 * h));
        set_e(&mut e_z, i_r, 0, -(v(i_r, 1) - v(i_r, 0)) / h);
    }

    // Top edge: 1 ≤ i_r ≤ N_r−2, i_z = N_z−1.
    // E_r: central; E_z: backward one-sided.
    for i_r in 1..n_r - 1 {
        set_e(&mut e_r, i_r, n_z - 1, -(v(i_r + 1, n_z - 1) - v(i_r - 1, n_z - 1)) / (2.0 * h));
        set_e(&mut e_z, i_r, n_z - 1, -(v(i_r, n_z - 1) - v(i_r, n_z - 2)) / h);
    }

    // Corners — one-sided in both directions; E_r = 0 at axis corners.
    // (0, 0)
    set_e(&mut e_z, 0, 0, -(v(0, 1) - v(0, 0)) / h);
    // (0, N_z−1)
    set_e(&mut e_z, 0, n_z - 1, -(v(0, n_z - 1) - v(0, n_z - 2)) / h);
    // (N_r−1, 0)
    set_e(&mut e_r, n_r - 1, 0, -(v(n_r - 1, 0) - v(n_r - 2, 0)) / h);
    set_e(&mut e_z, n_r - 1, 0, -(v(n_r - 1, 1) - v(n_r - 1, 0)) / h);
    // (N_r−1, N_z−1)
    set_e(&mut e_r, n_r - 1, n_z - 1, -(v(n_r - 1, n_z - 1) - v(n_r - 2, n_z - 1)) / h);
    set_e(&mut e_z, n_r - 1, n_z - 1, -(v(n_r - 1, n_z - 1) - v(n_r - 1, n_z - 2)) / h);

    (e_r, e_z)
}
