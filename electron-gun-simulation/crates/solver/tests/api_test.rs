use solver::{
    compute_electric_field, solve_laplace_cylindrical, Cell, Grid, Mask, SolverError,
};

fn grid(n_r: usize, n_z: usize) -> Grid {
    Grid::new(n_r, n_z, 1e-3)
}

fn mask(n_r: usize, n_z: usize) -> Mask {
    Mask::new(n_r, n_z)
}

/// Marks all outer boundary points as Fixed.
fn fix_outer_boundary(potential: &Grid, mask: &mut Mask) {
    let n_r = potential.n_r;
    let n_z = potential.n_z;
    for i_r in 0..n_r {
        mask.data[potential.idx(i_r, 0)] = Cell::Fixed;
        mask.data[potential.idx(i_r, n_z - 1)] = Cell::Fixed;
    }
    for i_z in 0..n_z {
        mask.data[potential.idx(n_r - 1, i_z)] = Cell::Fixed;
    }
}

// --- solve_laplace_cylindrical ---

#[test]
fn solve_valid_inputs_returns_ok() {
    let mut potential = grid(5, 5);
    let mut mask = mask(5, 5);
    fix_outer_boundary(&potential, &mut mask);
    let result = solve_laplace_cylindrical(&mut potential, &mask);
    assert!(result.is_ok());
}

#[test]
fn solve_dimension_mismatch() {
    let result = solve_laplace_cylindrical(&mut grid(5, 5), &mask(4, 5));
    assert!(matches!(result, Err(SolverError::DimensionMismatch { .. })));
}

#[test]
fn solve_grid_too_small_r() {
    let result = solve_laplace_cylindrical(&mut grid(2, 5), &mask(2, 5));
    assert!(matches!(result, Err(SolverError::GridTooSmall { .. })));
}

#[test]
fn solve_grid_too_small_z() {
    let result = solve_laplace_cylindrical(&mut grid(5, 2), &mask(5, 2));
    assert!(matches!(result, Err(SolverError::GridTooSmall { .. })));
}

// --- outer boundary validation ---

#[test]
fn solve_boundary_not_fixed_on_bottom() {
    let mut potential = grid(5, 5);
    let mut mask = mask(5, 5);
    fix_outer_boundary(&potential, &mut mask);
    // Free one cell on the bottom row.
    mask.data[potential.idx(2, 0)] = Cell::Free;
    let result = solve_laplace_cylindrical(&mut potential, &mask);
    assert!(matches!(
        result,
        Err(SolverError::InvalidOuterBoundary { i_z: 0, .. })
    ));
}

#[test]
fn solve_boundary_not_fixed_on_top() {
    let mut potential = grid(5, 5);
    let mut mask = mask(5, 5);
    fix_outer_boundary(&potential, &mut mask);
    // Free one cell on the top row.
    mask.data[potential.idx(2, 4)] = Cell::Free;
    let result = solve_laplace_cylindrical(&mut potential, &mask);
    assert!(matches!(
        result,
        Err(SolverError::InvalidOuterBoundary { i_z: 4, .. })
    ));
}

#[test]
fn solve_boundary_not_fixed_on_outer_radius() {
    let mut potential = grid(5, 5);
    let mut mask = mask(5, 5);
    fix_outer_boundary(&potential, &mut mask);
    // Free one interior-z cell on the outer radial column.
    mask.data[potential.idx(4, 2)] = Cell::Free;
    let result = solve_laplace_cylindrical(&mut potential, &mask);
    assert!(matches!(
        result,
        Err(SolverError::InvalidOuterBoundary { i_r: 4, .. })
    ));
}

// --- physics correctness ---

// Two coaxial cylinders: a solid inner conductor (i_r=0..=a, held at v_a) and an outer shell at
// i_r=b (held at v_b). The exact solution to cylindrical Laplace in the gap is:
//
//   V(r) = v_a + (v_b - v_a) * ln(r / a) / ln(b / a)
//
// All boundary cells are pre-loaded with this analytical value so the solver's only job is to
// find the correct potential at the Free interior cells.  Cells inside the inner conductor
// (i_r < a) are left Free to exercise the axis stencil; they should converge to v_a since they
// are enclosed by a uniform-potential boundary.
#[test]
fn solve_concentric_cylinders_matches_analytical() {
    let n_r = 11_usize;
    let n_z = 11_usize;
    let a = 3_usize; // inner electrode surface (outer surface of the inner conductor)
    let b = n_r - 1; // outer electrode index (= 10)
    let v_a = 1.0_f64;
    let v_b = 0.0_f64;

    let analytical = |i_r: usize| -> f64 {
        v_a + (v_b - v_a) * (i_r as f64 / a as f64).ln() / (b as f64 / a as f64).ln()
    };

    let mut potential = Grid::new(n_r, n_z, 1e-3);
    let mut mask = Mask::new(n_r, n_z);

    // Inner electrode shell: i_r = a, all i_z.
    for i_z in 0..n_z {
        let idx = potential.idx(a, i_z);
        potential.data[idx] = v_a;
        mask.data[idx] = Cell::Fixed;
    }

    // Outer electrode: i_r = b, all i_z.
    for i_z in 0..n_z {
        let idx = potential.idx(b, i_z);
        potential.data[idx] = v_b;
        mask.data[idx] = Cell::Fixed;
    }

    // z-boundaries: analytical value in the gap; v_a inside the conductor.
    for i_z in [0, n_z - 1] {
        for i_r in 0..n_r {
            let idx = potential.idx(i_r, i_z);
            potential.data[idx] = if i_r < a { v_a } else { analytical(i_r) };
            mask.data[idx] = Cell::Fixed;
        }
    }

    // All other cells are Free (i_r=0..a-1 inside conductor, i_r=a+1..b-1 in the gap).

    solve_laplace_cylindrical(&mut potential, &mut mask).unwrap();

    // Cells inside the conductor should converge to v_a (uniform potential).
    for i_z in 1..n_z - 1 {
        for i_r in 0..a {
            let v = potential.data[potential.idx(i_r, i_z)];
            assert!(
                (v - v_a).abs() < 1e-6,
                "inner conductor cell (i_r={i_r}, i_z={i_z}): expected {v_a}, got {v:.9}",
            );
        }
    }

    // Gap cells should match the logarithmic analytical solution.
    // Stencil truncation error for V=A*ln(r)+B is O(1/i_r^3); worst case at i_r=a+1=4 is ~0.003.
    for i_z in 1..n_z - 1 {
        for i_r in a + 1..b {
            let v = potential.data[potential.idx(i_r, i_z)];
            let v_exact = analytical(i_r);
            assert!(
                (v - v_exact).abs() < 1e-2,
                "gap cell (i_r={i_r}, i_z={i_z}): expected {v_exact:.6}, got {v:.6}",
            );
        }
    }
}

// --- compute_electric_field ---

#[test]
fn electric_field_dimensions_match_potential() {
    let potential = grid(7, 11);
    let (e_r, e_z) = compute_electric_field(&potential);
    assert_eq!((e_r.n_r, e_r.n_z), (7, 11));
    assert_eq!((e_z.n_r, e_z.n_z), (7, 11));
}

#[test]
fn electric_field_spacing_matches_potential() {
    let potential = grid(5, 5);
    let (e_r, e_z) = compute_electric_field(&potential);
    assert_eq!(e_r.h_m, potential.h_m);
    assert_eq!(e_z.h_m, potential.h_m);
}

// All finite-difference stencils (central and one-sided) are exact for linear
// potentials, so these tests hold at every grid point including edges and corners.

#[test]
fn electric_field_uniform_z() {
    // V[i_r, i_z] = i_z => E_z = -1 everywhere, E_r = 0 everywhere.
    let n_r = 4;
    let n_z = 5;
    let h = 1.0_f64;
    let mut potential = Grid::new(n_r, n_z, h);
    for i_z in 0..n_z {
        for i_r in 0..n_r {
            let k = potential.idx(i_r, i_z);
            potential.data[k] = i_z as f64;
        }
    }
    let (e_r, e_z) = compute_electric_field(&potential);
    for i_z in 0..n_z {
        for i_r in 0..n_r {
            let k = e_r.idx(i_r, i_z);
            assert!(
                e_r.data[k].abs() < 1e-12,
                "E_r non-zero at ({i_r},{i_z}): {}",
                e_r.data[k],
            );
            assert!(
                (e_z.data[k] + 1.0).abs() < 1e-12,
                "E_z wrong at ({i_r},{i_z}): {}",
                e_z.data[k],
            );
        }
    }
}

#[test]
fn electric_field_uniform_r() {
    // V[i_r, i_z] = i_r => E_r = -1 for i_r >= 1 (axis forced to 0), E_z = 0 everywhere.
    let n_r = 5;
    let n_z = 4;
    let h = 1.0_f64;
    let mut potential = Grid::new(n_r, n_z, h);
    for i_z in 0..n_z {
        for i_r in 0..n_r {
            let k = potential.idx(i_r, i_z);
            potential.data[k] = i_r as f64;
        }
    }
    let (e_r, e_z) = compute_electric_field(&potential);
    for i_z in 0..n_z {
        for i_r in 0..n_r {
            let k = e_r.idx(i_r, i_z);
            let expected_r = if i_r == 0 { 0.0 } else { -1.0 };
            assert!(
                (e_r.data[k] - expected_r).abs() < 1e-12,
                "E_r wrong at ({i_r},{i_z}): {} (expected {expected_r})",
                e_r.data[k],
            );
            assert!(
                e_z.data[k].abs() < 1e-12,
                "E_z non-zero at ({i_r},{i_z}): {}",
                e_z.data[k],
            );
        }
    }
}

#[test]
fn electric_field_constant_potential() {
    // V = 5 everywhere => E_r = E_z = 0 everywhere.
    let n_r = 4;
    let n_z = 4;
    let h = 1.0_f64;
    let mut potential = Grid::new(n_r, n_z, h);
    for v in potential.data.iter_mut() {
        *v = 5.0;
    }
    let (e_r, e_z) = compute_electric_field(&potential);
    for i_z in 0..n_z {
        for i_r in 0..n_r {
            let k = e_r.idx(i_r, i_z);
            assert!(
                e_r.data[k].abs() < 1e-12,
                "E_r non-zero at ({i_r},{i_z}): {}",
                e_r.data[k],
            );
            assert!(
                e_z.data[k].abs() < 1e-12,
                "E_z non-zero at ({i_r},{i_z}): {}",
                e_z.data[k],
            );
        }
    }
}
