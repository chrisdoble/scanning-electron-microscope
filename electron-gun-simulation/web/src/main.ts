import init, { GunParameters, solve_electron_gun } from 'wasm-api';

await init();

const params = new GunParameters();
const solution = solve_electron_gun(params);

console.log('GunSolution', {
  n_r: solution.n_r,
  n_z: solution.n_z,
  h_m: solution.h_m,
  iterations: solution.iterations,
  potential_v_length: solution.potential_v.length,
  e_r_v_per_m_length: solution.e_r_v_per_m.length,
  e_z_v_per_m_length: solution.e_z_v_per_m.length,
  mask_length: solution.mask.length,
});
