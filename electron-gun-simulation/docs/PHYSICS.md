# Physics and Numerics

This document specifies the physical model, discretisation, and numerical method used by the solver. It is the authoritative reference for anything in `crates/solver/`. If code and this document disagree, one of them is wrong — fix it before moving on.

Units are SI throughout (metres, volts, seconds, coulombs). Geometry inputs from the UI are in millimetres and converted at the wasm-api boundary.

## 1. Physical model

### 1.1 Assumptions

- **Electrostatic.** We solve for the steady-state potential. No time dependence, no magnetic fields, no radiation.
- **Axisymmetric.** The gun geometry is rotationally symmetric about the z-axis, so ∂/∂φ = 0 and the problem reduces to 2D in (r, z).
- **Charge-free interior.** We neglect space charge from the electron beam itself. This is a good approximation for the low beam currents typical of a thermionic gun at the potential-mapping stage; it would need revisiting for high-current applications. Consequently the governing equation is Laplace's, not Poisson's.
- **Perfect conductors.** Electrodes (filament, Wehnelt, anode) are equipotentials held at fixed voltages. Their surfaces are Dirichlet boundaries.
- **Vacuum everywhere else.** Permittivity is ε₀ uniformly; no dielectrics.

### 1.2 Governing equation

In free space with no charges, the electric potential V satisfies Laplace's equation:

    ∇²V = 0

In cylindrical coordinates (r, φ, z) with axisymmetry (∂/∂φ = 0):

    (1/r) ∂/∂r (r ∂V/∂r) + ∂²V/∂z² = 0

Expanding the r-derivative:

    ∂²V/∂r² + (1/r) ∂V/∂r + ∂²V/∂z² = 0              (1)

Equation (1) is the form we discretise. The (1/r) term is the source of the r=0 special case in §2.4.

### 1.3 Electric field

Once V is solved, **E** = −∇V. In axisymmetric cylindrical coordinates:

    E_r = −∂V/∂r
    E_z = −∂V/∂z
    E_φ = 0

These are computed on the same grid by central differences after the solve (§2.6).

## 2. Discretisation

### 2.1 Grid

A uniform rectangular grid of points over the (r, z) domain. Each point corresponds to a specific location in space where the potential V is stored.

- r ∈ [0, R_max], N_r points (indexed i_r = 0 to N_r−1), dr = R_max / (N_r − 1)
- z ∈ [0, Z_max], N_z points (indexed i_z = 0 to N_z−1), dz = Z_max / (N_z − 1)

The z-axis points upward, as in standard cylindrical coordinates: i_z=0 corresponds to z=0 (bottom of the domain), i_z=N_z−1 corresponds to z=Z_max (top). When rendering to screen, the display is flipped vertically so that z=0 appears at the bottom of the canvas.

The physical coordinates of point (i_r, i_z) are:

    r = i_r · dr
    z = i_z · dz

So the point at i_r=0, i_z=0 corresponds to (r=0, z=0), and the point at i_r=N_r−1, i_z=N_z−1 corresponds to (r=R_max, z=Z_max).

**Decision: dr = dz = h**, where h = min(smallest_geometry_dimension) / 10. Setting dr = dz avoids carrying separate dr² and dz² denominators through the stencil, reducing the number of distinct coefficients in equations (2) and (4). The grid spans a box sized to contain the full gun plus a margin (see §3.2).

### 2.2 Memory layout

Potential is stored in a row-major `Vec<f64>` of length N_r × N_z. We treat z as the row index and r as the column index. The linear index for point (i_r, i_z) is:

    V[i_z * N_r + i_r]

This makes r the fast axis (stride 1): incrementing i_r by 1 moves one element forward in memory, while incrementing i_z by 1 jumps N_r elements. Sequential access along a row (fixed z, varying r) is cache-friendly. **This layout is fixed and assumed everywhere.**

Note that because z points upward, row i_z=0 in the array corresponds to the _bottom_ of the physical domain. This is the standard convention in scientific computing (and matches how, e.g., image data is stored in many formats). The visualisation layer handles the vertical flip for display.

### 2.3 Interior stencil (r > 0)

In this section and §2.4, we use shorthand i for i_r and j for i_z to keep equations readable.

Second-order central finite differences applied to equation (1):

    ∂²V/∂r² ≈ (V[i+1, j] − 2 V[i, j] + V[i−1, j]) / h²
    ∂²V/∂z² ≈ (V[i, j+1] − 2 V[i, j] + V[i, j−1]) / h²
    ∂V/∂r  ≈ (V[i+1, j] − V[i−1, j]) / (2h)

with r = i · h. Substituting into (1) and multiplying through by h²:

    V[i+1, j] (1 + 1/(2i))
  + V[i−1, j] (1 − 1/(2i))
  + V[i, j+1]
  + V[i, j−1]
  − 4 V[i, j] = 0                                       (2)

Equation (2) is the finite-difference equation for interior points at i ≥ 1. Each Free point contributes one equation of this form to the linear system.

### 2.4 Axis (r = 0) — the special case

At r = 0 the term (1/r) ∂V/∂r in equation (1) is formally 0/0. Resolve by L'Hôpital: as r → 0, ∂V/∂r → 0 by symmetry, so (1/r) ∂V/∂r → ∂²V/∂r². Equation (1) at r=0 becomes:

    2 ∂²V/∂r² + ∂²V/∂z² = 0                            (3)

Discretising: because of axisymmetry, V(−h, z) = V(+h, z), so the central second difference at r=0 becomes:

    ∂²V/∂r²|_{r=0} ≈ 2 (V[1, j] − V[0, j]) / h²

Substituting into (3) and multiplying through by h²:

    4 V[1, j]
  + V[0, j+1]
  + V[0, j−1]
  − 6 V[0, j] = 0                                       (4)

Equation (4) is the finite-difference equation for i = 0. **This must be treated explicitly as a separate code path — applying equation (2) at i=0 is a bug that silently produces wrong answers near the axis**, which is exactly where the beam lives. Tests must cover this.

### 2.5 Boundary conditions

- **Electrode surfaces (Dirichlet).** Points that lie on the filament, Wehnelt, or anode are held at the electrode voltage and never updated during iteration. These points are stored in the potential array at their fixed voltage and marked in the mask as `Fixed`. See §3.1 for rasterisation.
- **Axis (r = 0).** Neumann ∂V/∂r = 0 is enforced implicitly by the symmetric stencil in equation (4). No separate code needed.
- **Outer radial boundary (r = R_max).** **Decision: Dirichlet V = 0.** These boundary points are stored in the array as 0.0 and marked `Fixed(0.0)`. Valid only if R_max is large enough that the true potential has decayed to ≈0 there. See §3.2 on box sizing. An alternative is Neumann ∂V/∂n = 0 (field lines parallel to the boundary), which is more forgiving of a tight box but less accurate for a localised charge distribution. We start with Dirichlet; revisit if box-size sensitivity studies show problems.
- **Axial boundaries (z = 0 and z = Z_max).** **Decision: Dirichlet V = 0** for the same reason.

### 2.6 Electric field extraction

After the potential solve, compute **E** on the same grid. In all formulas below, i is shorthand for i_r and j for i_z.

**Interior** (1 ≤ i ≤ N_r−2, 1 ≤ j ≤ N_z−2) — central differences:

    E_r[i, j] = −(V[i+1, j] − V[i−1, j]) / (2h)
    E_z[i, j] = −(V[i, j+1] − V[i, j−1]) / (2h)

**Axis** (i = 0, 1 ≤ j ≤ N_z−2):

    E_r[0, j] = 0
    E_z[0, j] = −(V[0, j+1] − V[0, j−1]) / (2h)

E_r = 0 because V is symmetric about the axis (V is an even function of r, so ∂V/∂r = 0 at r = 0). E_z uses the same central difference as interior points since both z-neighbours exist.

**Outer radial edge** (i = N_r−1, 1 ≤ j ≤ N_z−2) — first-order one-sided (backward) difference for E_r, central for E_z:

    E_r[N_r−1, j] = −(V[N_r−1, j] − V[N_r−2, j]) / h
    E_z[N_r−1, j] = −(V[N_r−1, j+1] − V[N_r−1, j−1]) / (2h)

**Bottom edge** (j = 0, 1 ≤ i ≤ N_r−2) — central for E_r, first-order one-sided (forward) for E_z:

    E_r[i, 0] = −(V[i+1, 0] − V[i−1, 0]) / (2h)
    E_z[i, 0] = −(V[i, 1] − V[i, 0]) / h

**Top edge** (j = N_z−1, 1 ≤ i ≤ N_r−2) — central for E_r, first-order one-sided (backward) for E_z:

    E_r[i, N_z−1] = −(V[i+1, N_z−1] − V[i−1, N_z−1]) / (2h)
    E_z[i, N_z−1] = −(V[i, N_z−1] − V[i, N_z−2]) / h

**Corners** (four points where two edges meet) — first-order one-sided in both directions:

    (i=0, j=0):           E_r = 0,                                    E_z = −(V[0, 1] − V[0, 0]) / h
    (i=0, j=N_z−1):       E_r = 0,                                    E_z = −(V[0, N_z−1] − V[0, N_z−2]) / h
    (i=N_r−1, j=0):       E_r = −(V[N_r−1, 0] − V[N_r−2, 0]) / h,   E_z = −(V[N_r−1, 1] − V[N_r−1, 0]) / h
    (i=N_r−1, j=N_z−1):   E_r = −(V[N_r−1, N_z−1] − V[N_r−2, N_z−1]) / h,   E_z = −(V[N_r−1, N_z−1] − V[N_r−1, N_z−2]) / h

These edge and corner formulas are only first-order accurate (O(h)), but the domain edges are deliberately placed far from the electrodes where the field is small, so the reduced accuracy is inconsequential.

**E** is computed once after the solve, not during the solve.

## 3. Geometry rasterisation

### 3.1 Electrode mask

Before solving, build a mask array of the same dimensions as V. Each point is either `Free` (solver updates it) or `Fixed(voltage)` (Dirichlet boundary, not updated).

Rasterisation rule: a point (i_r, i_z) is `Fixed` if its physical location (r = i_r·h, z = i_z·h) lies inside any electrode's 2D cross-section. For overlapping electrodes, later electrodes win (document order matters — in practice they shouldn't overlap, and we should assert this).

Electrodes and their cross-sections in (r, z):

- **Filament.** Modelled as a small disk or short cylinder at the specified position. For a hairpin tungsten filament the axisymmetric approximation is crude but adequate for gross field shape.
- **Wehnelt cylinder.** A cylindrical cup, open at the back (where the filament enters) and closed at the front with an aperture (where the beam exits). In the (r, z) cross-section this is an L-shaped region: the cylindrical wall (outer radius, inner radius, height) plus the front cap (a flat annular plate with some thickness, with a central aperture of a given radius). Biased negative relative to the filament by a few hundred volts.
- **Anode.** A flat plate with a central aperture, positioned downstream (higher z) of the Wehnelt. Held at 0 V (ground) — the filament and Wehnelt sit at large negative voltages.

Exact parameterisation (variable names, units, defaults) is defined in the `GunParameters` struct in `ARCHITECTURE.md`.

### 3.2 Domain sizing

The simulation box must extend far enough that the Dirichlet V=0 far-field boundary condition is a good approximation. Heuristic starting point: box extends at least 3× the largest electrode dimension in every direction beyond the electrodes themselves. Verify with box-size sensitivity studies (solve with 2×, 3×, 5× margin and check that the solution in the electrode region is insensitive).

## 4. Solver

### 4.1 Method

**Decision: Sparse direct solve via Cholesky factorisation.** The discretised Laplace equation forms a sparse linear system **Ax = b** where each Free point contributes one equation (from equation (2) or (4)) and one unknown (the potential at that point). The system is solved in one shot by sparse Cholesky factorisation, which is vastly faster than iterative methods (SOR, Gauss-Seidel) for grids of this size.

### 4.2 Linear system assembly

Number the Free points 0 through N_free−1. This numbering maps each Free grid point (i_r, i_z) to an equation/variable index k. The mapping must be built before assembly and is used to translate between grid coordinates and matrix indices.

For each Free point k at grid position (i, j):

**If i > 0** (interior, equation (2)):

The stencil coefficients are:

    self:       −4
    r+1 (i+1, j):  1 + 1/(2i)
    r−1 (i−1, j):  1 − 1/(2i)
    z+1 (i, j+1):  1
    z−1 (i, j−1):  1

**If i = 0** (axis, equation (4)):

The stencil coefficients are:

    self:       −6
    r+1 (1, j):     4
    z+1 (0, j+1):   1
    z−1 (0, j−1):   1

(There is no r−1 neighbour at i=0; the symmetric ghost point is already folded into the coefficient on r+1.)

For each neighbour in the stencil:
- If the neighbour is **Free**: add `coefficient` to matrix entry A[k, neighbour_k] where neighbour_k is the Free-point index of the neighbour.
- If the neighbour is **Fixed** (a Dirichlet boundary with known voltage V_boundary): move its contribution to the right-hand side: `b[k] -= coefficient * V_boundary`.

The diagonal entry A[k, k] is always the self-coefficient (−4 or −6).

### 4.3 Matrix properties

The matrix A as assembled above has negative diagonal and non-negative off-diagonal entries. To obtain a symmetric positive definite (SPD) matrix suitable for Cholesky factorisation, negate the entire system: solve **(-A)x = -b**. The negated matrix -A has positive diagonal and non-positive off-diagonal entries and is SPD by the properties of the discrete Laplacian.

The matrix is sparse: each row has at most 5 non-zero entries (the point itself and up to 4 neighbours). For a 452 × 952 grid with ~400,000 Free points, the matrix has ~2 million non-zero entries total.

### 4.4 Solving

1. Build the sparse matrix in triplet (COO) format: a list of (row, col, value) entries.
2. Convert to compressed sparse column (CSC) format as required by `faer`.
3. Compute the Cholesky factorisation of -A.
4. Solve for x using the factorisation and -b.
5. Write the solution values back into the potential grid at the corresponding Free points.

The factorisation is the expensive step (typically 0.5–2 seconds for this grid size). The back-substitution is cheap (milliseconds). Both happen once per solve — there is no iteration.

### 4.5 Symmetry verification

The stencil for i > 0 is not obviously symmetric: the coefficient on the r+1 neighbour is (1 + 1/(2i)) while the coefficient on the r−1 neighbour is (1 − 1/(2i)). However, the assembled matrix **is** symmetric because the entry A[k, k'] (from point k referencing neighbour k') equals A[k', k] (from point k' referencing neighbour k). This can be verified:

Point at index i references its r+1 neighbour at index i+1 with coefficient (1 + 1/(2i)). Point at index i+1 references its r−1 neighbour at index i with coefficient (1 − 1/(2(i+1))). These are **not** equal, so the raw stencil does **not** produce a symmetric matrix.

To obtain a symmetric system, multiply each equation by a symmetrising weight. For the cylindrical Laplacian, the appropriate weight for the equation at grid index i is `w(i) = r = i · h` (or equivalently just `i`, since h is a constant factor). For i = 0, the weight from the L'Hôpital-derived stencil is handled separately.

**Weighted stencil for i > 0:** Multiply equation (2) by i:

    i · (1 + 1/(2i)) V[i+1, j] + i · (1 − 1/(2i)) V[i−1, j] + i · V[i, j+1] + i · V[i, j−1] − 4i · V[i, j] = 0

Simplifying:

    (i + 1/2) V[i+1, j] + (i − 1/2) V[i−1, j] + i · V[i, j+1] + i · V[i, j−1] − 4i · V[i, j] = 0     (5)

Now check symmetry: point i references i+1 with coefficient (i + 1/2). Point i+1 references i with coefficient ((i+1) − 1/2) = (i + 1/2). These match — the matrix is symmetric.

**Weighted stencil for i = 0:** The r = 0 equation (4) is already self-consistent (it has no r−1 neighbour), but its coefficients must be compatible with the i = 1 equation's reference back to i = 0. The i = 1 weighted equation references i = 0 with coefficient (1 − 1/2) = 1/2. So the i = 0 equation must reference i = 1 with coefficient 1/2. Scale equation (4) by 1/8:

    (1/2) V[1, j] + (1/8) V[0, j+1] + (1/8) V[0, j−1] − (6/8) V[0, j] = 0                             (6)

Check: point i=0 references i=1 with coefficient 1/2. Point i=1 references i=0 with coefficient (1 − 1/2) = 1/2. Symmetric.

**Summary of weighted stencil coefficients for matrix assembly:**

For i > 0 (equation (5)):

    self:           −4i
    r+1 (i+1, j):   i + 1/2
    r−1 (i−1, j):   i − 1/2
    z+1 (i, j+1):   i
    z−1 (i, j−1):   i

For i = 0 (equation (6)):

    self:           −6/8
    r+1 (1, j):      1/2
    z+1 (0, j+1):    1/8
    z−1 (0, j−1):    1/8

Use these weighted coefficients when assembling the sparse matrix. The resulting matrix is symmetric positive definite (after negation) and suitable for Cholesky factorisation.

## 5. Out of scope

The following are explicitly not part of the current solver. They are listed here so that they are not added without a deliberate decision:

- Space charge (Poisson's equation instead of Laplace's) for high-current regimes.
- Electron trajectory tracing through the solved **E** field.
- Adaptive mesh refinement near electrode edges and apertures.
- Full 3D (non-axisymmetric) geometries.

## 6. Symbols

| Symbol   | Meaning                                     | Units |
| -------- | ------------------------------------------- | ----- |
| V        | Electric potential                          | V     |
| **E**    | Electric field                              | V/m   |
| r, z     | Cylindrical coordinates                     | m     |
| h        | Grid spacing (dr = dz)                      | m     |
| N_r      | Number of grid points along r               | —     |
| N_z      | Number of grid points along z               | —     |
| N_free   | Number of Free grid points                  | —     |
| i_r, i_z | Grid point indices (r, z direction)         | —     |
| i, j     | Shorthand for i_r, i_z in stencil equations | —     |
| k        | Linear index into the Free-point numbering  | —     |
| A        | Sparse coefficient matrix (N_free × N_free) | —     |
| b        | Right-hand side vector (length N_free)      | V     |
| x        | Solution vector (length N_free)             | V     |
| ε₀       | Vacuum permittivity                         | F/m   |
