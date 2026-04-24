import init, { GunParameters } from 'wasm-api';
import { buildControls } from './sliders';
import type { AppState } from './state';

await init();

const state: AppState = {
  gunParameters: new GunParameters(),
};

buildControls(state, () => {
  console.log('GunParameters updated', {
    filament_radius_mm: state.gunParameters.filament_radius_mm,
    filament_voltage_v: state.gunParameters.filament_voltage_v,
    wehnelt_bias_v: state.gunParameters.wehnelt_bias_v,
    anode_voltage_v: state.gunParameters.anode_voltage_v,
  });
});
