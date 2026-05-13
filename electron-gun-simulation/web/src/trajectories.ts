import type { GunSolution, GunParameters } from './worker-protocol';
import { type ViewState, solutionToContainer } from './view';

const CHARGE_OVER_MASS = 1.602176634e-19 / 9.1093837015e-31; // C/kg (e/m_e)
const MAX_STEPS = 10_000;

// Interleaved (r_index, z_index) pairs using continuous (non-integer) grid indices.
export type Trajectory = Float32Array;

function bilinear(
  data: Float64Array,
  n_r: number, n_z: number,
  fi_r: number, fi_z: number,
): number {
  const r0 = Math.floor(fi_r);
  const z0 = Math.floor(fi_z);
  if (r0 < 0 || r0 >= n_r - 1 || z0 < 0 || z0 >= n_z - 1) return 0;
  const dr = fi_r - r0;
  const dz = fi_z - z0;
  return (
    data[z0 * n_r + r0]       * (1 - dr) * (1 - dz) +
    data[z0 * n_r + r0 + 1]   * dr       * (1 - dz) +
    data[(z0 + 1) * n_r + r0] * (1 - dr) * dz +
    data[(z0 + 1) * n_r + r0 + 1] * dr   * dz
  );
}

function integrateOne(solution: GunSolution, r0: number, z0: number, dt: number): Trajectory {
  const { n_r, n_z, h_m, z_lo_m, e_r_v_per_m, e_z_v_per_m, mask } = solution;

  let r = r0;
  let z = z0;
  let vr = 0.0;
  let vz = 0.0;

  // Pre-allocate a buffer; shrink to actual length at the end.
  const buf = new Float32Array(MAX_STEPS * 2);
  let nPoints = 0;

  for (let step = 0; step < MAX_STEPS; step++) {
    const fi_r = r / h_m;
    const fi_z = (z - z_lo_m) / h_m;

    // Stop if out of domain or inside an electrode.
    const ir = Math.round(fi_r);
    const iz = Math.round(fi_z);
    if (ir < 0 || ir >= n_r || iz < 0 || iz >= n_z) break;
    if (mask[iz * n_r + ir] === 1) break;

    buf[nPoints * 2]     = fi_r;
    buf[nPoints * 2 + 1] = fi_z;
    nPoints++;

    // Acceleration function at a given physical position.
    const ar = (rp: number, zp: number): [number, number] => {
      const fir = rp / h_m;
      const fiz = (zp - z_lo_m) / h_m;
      const er = bilinear(e_r_v_per_m, n_r, n_z, fir, fiz);
      const ez = bilinear(e_z_v_per_m, n_r, n_z, fir, fiz);
      return [-CHARGE_OVER_MASS * er, -CHARGE_OVER_MASS * ez];
    };

    // RK4 for state [r, z, vr, vz].
    const [k1vr, k1vz] = ar(r, z);
    const k1r = vr;  const k1z = vz;

    const [k2vr, k2vz] = ar(r + 0.5 * dt * k1r, z + 0.5 * dt * k1z);
    const k2r = vr + 0.5 * dt * k1vr;  const k2z = vz + 0.5 * dt * k1vz;

    const [k3vr, k3vz] = ar(r + 0.5 * dt * k2r, z + 0.5 * dt * k2z);
    const k3r = vr + 0.5 * dt * k2vr;  const k3z = vz + 0.5 * dt * k2vz;

    const [k4vr, k4vz] = ar(r + dt * k3r, z + dt * k3z);
    const k4r = vr + dt * k3vr;  const k4z = vz + dt * k3vz;

    r  += dt / 6 * (k1r  + 2 * k2r  + 2 * k3r  + k4r);
    z  += dt / 6 * (k1z  + 2 * k2z  + 2 * k3z  + k4z);
    vr += dt / 6 * (k1vr + 2 * k2vr + 2 * k3vr + k4vr);
    vz += dt / 6 * (k1vz + 2 * k2vz + 2 * k3vz + k4vz);

    // Axis reflection: electrons cannot have r < 0.
    if (r < 0) { r = -r; vr = -vr; }
  }

  return buf.slice(0, nPoints * 2);
}

export function computeTrajectories(solution: GunSolution, parameters: GunParameters): Trajectory[] {
  const { n_r, n_z, h_m, z_lo_m, mask } = solution;

  // Filament physical bounds in metres.
  const fil_r  = parameters.filament_radius_mm * 1e-3;
  const fil_z  = parameters.filament_z_mm      * 1e-3;
  const fil_t  = parameters.filament_thickness_mm * 1e-3;
  const fil_zlo = fil_z - fil_t / 2;
  const fil_zhi = fil_z + fil_t / 2;

  // Timestep: dt = 0.5 * h / v_max, where v_max is the speed of an electron
  // accelerated through the full potential difference.
  const delta_v = Math.abs(parameters.filament_voltage_v - parameters.anode_voltage_v);
  const v_max = Math.sqrt(2 * CHARGE_OVER_MASS * delta_v);
  const dt = v_max > 0 ? 0.5 * h_m / v_max : 1e-12;

  const trajectories: Trajectory[] = [];

  const NEIGHBORS = [[-1, 0], [1, 0], [0, -1], [0, 1]] as const;

  for (let i_z = 1; i_z < n_z - 1; i_z++) {
    for (let i_r = 0; i_r < n_r - 1; i_r++) {
      if (mask[i_z * n_r + i_r] !== 0) continue;

      // Is this free cell adjacent to a filament cell?
      const adjacentToFilament = NEIGHBORS.some(([dr, dz]) => {
        const nr = i_r + dr;
        const nz = i_z + dz;
        if (nr < 0 || nr >= n_r || nz < 0 || nz >= n_z) return false;
        if (mask[nz * n_r + nr] !== 1) return false;
        const phys_r = nr * h_m;
        const phys_z = z_lo_m + nz * h_m;
        return phys_r <= fil_r && phys_z >= fil_zlo && phys_z <= fil_zhi;
      });

      if (!adjacentToFilament) continue;

      const traj = integrateOne(solution, i_r * h_m, z_lo_m + i_z * h_m, dt);
      if (traj.length >= 4) trajectories.push(traj);
    }
  }

  return trajectories;
}

export function renderTrajectories(
  overlay: HTMLCanvasElement,
  trajectories: Trajectory[],
  solution: GunSolution,
  viewState: ViewState,
  fitScale: number,
  containerW: number,
  containerH: number,
): void {
  if (trajectories.length === 0) return;

  const ctx = overlay.getContext('2d');
  if (ctx === null) return;

  const { n_r, n_z } = solution;
  const canvasW = 2 * n_r - 1;
  const canvasH = n_z;
  const axisX = n_r - 1; // canvas x of the symmetry axis (r = 0)

  ctx.strokeStyle = '#000';
  ctx.lineWidth = 1;

  for (const traj of trajectories) {
    const nPoints = traj.length / 2;
    if (nPoints < 2) continue;

    // Draw right half and left-half mirror in a single pass each.
    for (const sign of [1, -1] as const) {
      ctx.beginPath();
      let started = false;

      for (let i = 0; i < nPoints; i++) {
        const fi_r = traj[i * 2];
        const fi_z = traj[i * 2 + 1];

        // Canvas pixel coordinates.
        const canvasX = axisX + sign * fi_r;
        const canvasY = (n_z - 1) - fi_z;

        const { cx, cy } = solutionToContainer(
          canvasX, canvasY, viewState, fitScale,
          containerW, containerH, canvasW, canvasH,
        );

        if (!started) { ctx.moveTo(cx, cy); started = true; }
        else ctx.lineTo(cx, cy);
      }

      ctx.stroke();

      // Skip duplicate left-half rendering for axis trajectories (fi_r ≈ 0).
      if (traj[0] < 0.5) break;
    }
  }
}
