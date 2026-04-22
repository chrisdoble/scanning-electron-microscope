use solver::{Grid, Mask, SolverConfig, SolverError, compute_electric_field, solve_laplace_cylindrical};

fn grid(n_r: usize, n_z: usize) -> Grid {
    Grid::new(n_r, n_z, 1e-3)
}

fn mask(n_r: usize, n_z: usize) -> Mask {
    Mask::new(n_r, n_z)
}

// --- solve_laplace_cylindrical ---

#[test]
fn solve_valid_inputs_returns_ok() {
    let result = solve_laplace_cylindrical(&mut grid(5, 5), &mask(5, 5), &SolverConfig::default());
    assert!(result.is_ok());
}

#[test]
fn solve_dimension_mismatch() {
    let result = solve_laplace_cylindrical(&mut grid(5, 5), &mask(4, 5), &SolverConfig::default());
    assert!(matches!(result, Err(SolverError::DimensionMismatch { .. })));
}

#[test]
fn solve_grid_too_small_r() {
    let result = solve_laplace_cylindrical(&mut grid(2, 5), &mask(2, 5), &SolverConfig::default());
    assert!(matches!(result, Err(SolverError::GridTooSmall { .. })));
}

#[test]
fn solve_grid_too_small_z() {
    let result = solve_laplace_cylindrical(&mut grid(5, 2), &mask(5, 2), &SolverConfig::default());
    assert!(matches!(result, Err(SolverError::GridTooSmall { .. })));
}

#[test]
fn solve_omega_too_large() {
    let config = SolverConfig { omega: 2.0, ..SolverConfig::default() };
    let result = solve_laplace_cylindrical(&mut grid(5, 5), &mask(5, 5), &config);
    assert!(matches!(result, Err(SolverError::InvalidOmega(_))));
}

#[test]
fn solve_omega_zero() {
    let config = SolverConfig { omega: 0.0, ..SolverConfig::default() };
    let result = solve_laplace_cylindrical(&mut grid(5, 5), &mask(5, 5), &config);
    assert!(matches!(result, Err(SolverError::InvalidOmega(_))));
}

#[test]
fn solve_negative_tolerance() {
    let config = SolverConfig { tolerance_v: -1.0, ..SolverConfig::default() };
    let result = solve_laplace_cylindrical(&mut grid(5, 5), &mask(5, 5), &config);
    assert!(matches!(result, Err(SolverError::InvalidTolerance(_))));
}

#[test]
fn solve_zero_tolerance() {
    let config = SolverConfig { tolerance_v: 0.0, ..SolverConfig::default() };
    let result = solve_laplace_cylindrical(&mut grid(5, 5), &mask(5, 5), &config);
    assert!(matches!(result, Err(SolverError::InvalidTolerance(_))));
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
