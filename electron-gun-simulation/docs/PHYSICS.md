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

These are computed on the same grid by central differences after the solve converges (§2.6).

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

with r = i · h. Substituting into (1) and solving for V[i, j]:

    V[i, j] = (1/4) [ V[i+1, j] (1 + 1/(2i))
                     + V[i−1, j] (1 − 1/(2i))
                     + V[i, j+1]
                     + V[i, j−1] ]                       (2)

This is the update rule used by Gauss-Seidel for interior points at i ≥ 1.

### 2.4 Axis (r = 0) — the special case

At r = 0 the term (1/r) ∂V/∂r in equation (1) is formally 0/0. Resolve by L'Hôpital: as r → 0, ∂V/∂r → 0 by symmetry, so (1/r) ∂V/∂r → ∂²V/∂r². Equation (1) at r=0 becomes:

    2 ∂²V/∂r² + ∂²V/∂z² = 0                            (3)

Discretising: because of axisymmetry, V(−h, z) = V(+h, z), so the central second difference at r=0 becomes:

    ∂²V/∂r²|_{r=0} ≈ 2 (V[1, j] − V[0, j]) / h²

Substituting into (3):

    V[0, j] = (1/6) [ 4 V[1, j] + V[0, j+1] + V[0, j−1] ]   (4)

Equation (4) is the update rule for i = 0. **This must be treated explicitly as a separate code path — applying equation (2) at i=0 is a bug that silently produces wrong answers near the axis**, which is exactly where the beam lives. Tests must cover this.

### 2.5 Boundary conditions

- **Electrode surfaces (Dirichlet).** Points that lie on the filament, Wehnelt, or anode are held at the electrode voltage and never updated during iteration. These points are stored in the potential array at their fixed voltage and marked in the mask as `Fixed`. When a stencil at an adjacent interior point references a boundary neighbour, it simply reads the stored value — no special-case code needed near electrodes. See §3.1 for rasterisation.
- **Axis (r = 0).** Neumann ∂V/∂r = 0 is enforced implicitly by the symmetric stencil in equation (4). No separate code needed.
- **Outer radial boundary (r = R_max).** **Decision: Dirichlet V = 0.** These boundary points are stored in the array as 0.0 and marked `Fixed(0.0)`. Valid only if R_max is large enough that the true potential has decayed to ≈0 there. See §3.2 on box sizing. An alternative is Neumann ∂V/∂n = 0 (field lines parallel to the boundary), which is more forgiving of a tight box but less accurate for a localised charge distribution. We start with Dirichlet; revisit if box-size sensitivity studies show problems.
- **Axial boundaries (z = 0 and z = Z_max).** **Decision: Dirichlet V = 0** for the same reason.

### 2.6 Electric field extraction

After the potential solve converges, compute **E** on the same grid. In all formulas below, i is shorthand for i_r and j for i_z.

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

**E** is computed once after convergence, not inside the iteration loop.

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

**Decision: Successive Over-Relaxation (SOR).** Gauss-Seidel is simple and correct; SOR is Gauss-Seidel plus a relaxation factor ω ∈ (1, 2) that accelerates convergence substantially (often 10–50× for this class of problem). The update becomes:

    V_new[i, j] = (1 − ω) V_old[i, j] + ω · V_GS[i, j]

where V_GS is the right-hand side of equation (2) or (4). **Decision: start with ω = 1.9** and tune empirically; the optimal ω for a 2D Laplacian on an N×N grid is approximately 2 / (1 + π/N), which for N ~ 400 gives ω ≈ 1.985. If SOR performance becomes a bottleneck, consider switching to a direct sparse solve (e.g. via `faer`). The solver lives behind a trait in `crates/solver/` so swapping is cheap later.

### 4.2 Iteration and convergence

Iterate in-place over all `Free` points in row-major order (i.e. Gauss-Seidel, not Jacobi — we use updated values as soon as they're available).

Track convergence inline: each time a point is updated, compute the absolute change |V_new − V_old| and keep a running maximum across all points in the current iteration. At the end of each iteration, if this maximum residual is below the tolerance, stop. This adds negligible cost (one comparison per point per iteration).

**Decision: tol = 1e-6 × V_max**, where V_max is the largest electrode voltage magnitude. This makes the tolerance relative to the problem's voltage scale: both a 100 V gun and a 30 kV gun converge to the same relative precision (~1 part per million of V_max), rather than a fixed absolute threshold that would be unnecessarily tight for high-voltage guns or too loose for low-voltage ones.

Also impose a hard iteration cap (e.g. 50000) and return an error if hit — hitting the cap means something is wrong, not that more iterations would help.

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
| i_r, i_z | Grid point indices (r, z direction)         | —     |
| i, j     | Shorthand for i_r, i_z in stencil equations | —     |
| ω        | SOR relaxation factor                       | —     |
| ε₀       | Vacuum permittivity                         | F/m   |
