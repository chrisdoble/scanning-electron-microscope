import init, { GunParameters, solve_electron_gun } from 'wasm-api';
import type { GunSolution, WorkerResponse } from './worker-protocol';
import { WorkerRequestSchema } from './worker-protocol';

// A message that arrived before init() completed, if any. Only the most
// recent one is kept — the main thread always wants the latest params.
let pendingMessage: MessageEvent | null = null;
let initialized = false;

addEventListener('message', (message: MessageEvent) => {
  if (initialized) {
    handleMessage(message);
  } else {
    pendingMessage = message;
  }
});

await init();
initialized = true;

if (pendingMessage !== null) {
  handleMessage(pendingMessage);
  pendingMessage = null;
}

function handleMessage(event: MessageEvent): void {
  const parseResult = WorkerRequestSchema.safeParse(event.data);
  if (!parseResult.success) {
    const out: WorkerResponse = { type: 'error', message: `Invalid message: ${parseResult.error.message}` };
    postMessage(out);
    return;
  }
  const msg = parseResult.data;

  const p = new GunParameters();
  const src = msg.parameters;
  p.filament_radius_mm = src.filament_radius_mm;
  p.filament_thickness_mm = src.filament_thickness_mm;
  p.filament_z_mm = src.filament_z_mm;
  p.filament_voltage_v = src.filament_voltage_v;
  p.wehnelt_outer_radius_mm = src.wehnelt_outer_radius_mm;
  p.wehnelt_inner_radius_mm = src.wehnelt_inner_radius_mm;
  p.wehnelt_z_mm = src.wehnelt_z_mm;
  p.wehnelt_height_mm = src.wehnelt_height_mm;
  p.wehnelt_cap_thickness_mm = src.wehnelt_cap_thickness_mm;
  p.wehnelt_aperture_radius_mm = src.wehnelt_aperture_radius_mm;
  p.wehnelt_bias_v = src.wehnelt_bias_v;
  p.anode_z_mm = src.anode_z_mm;
  p.anode_thickness_mm = src.anode_thickness_mm;
  p.anode_outer_radius_mm = src.anode_outer_radius_mm;
  p.anode_aperture_radius_mm = src.anode_aperture_radius_mm;
  p.anode_voltage_v = src.anode_voltage_v;

  try {
    const result = solve_electron_gun(p);
    const solution: GunSolution = {
      n_r: result.n_r,
      n_z: result.n_z,
      h_m: result.h_m,
      iterations: result.iterations,
      potential_v: result.potential_v,
      e_r_v_per_m: result.e_r_v_per_m,
      e_z_v_per_m: result.e_z_v_per_m,
      mask: result.mask,
    };
    result.free();
    p.free();
    const out: WorkerResponse = { type: 'success', solution };
    postMessage(out, {
      transfer: [
        solution.potential_v.buffer,
        solution.e_r_v_per_m.buffer,
        solution.e_z_v_per_m.buffer,
        solution.mask.buffer,
      ],
    });
  } catch (err) {
    p.free();
    const out: WorkerResponse = { type: 'error', message: String(err) };
    postMessage(out);
  }
}
