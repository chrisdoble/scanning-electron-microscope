import type { GunParameters } from './worker-protocol';
import type { AppState } from './state';

type GunParameterKey = keyof GunParameters;

interface SliderDefinition {
  label: string;
  field: GunParameterKey;
  min: number;
  max: number;
  step: number;
  unit: string;
}

interface GroupDefinition {
  title: string;
  sliders: SliderDefinition[];
}

const GROUPS: GroupDefinition[] = [
  {
    title: 'Filament',
    sliders: [
      { label: 'Radius',     field: 'filament_radius_mm',    min: 0.05,   max: 1.0,   step: 0.01, unit: 'mm' },
      { label: 'Thickness',  field: 'filament_thickness_mm', min: 0.01,   max: 0.5,   step: 0.01, unit: 'mm' },
      { label: 'z position', field: 'filament_z_mm',         min: 1.0,    max: 20.0,  step: 0.1,  unit: 'mm' },
      { label: 'Voltage',    field: 'filament_voltage_v',    min: -30000, max: -1000, step: 100,  unit: 'V'  },
    ],
  },
  {
    title: 'Wehnelt cylinder',
    sliders: [
      { label: 'Outer radius',    field: 'wehnelt_outer_radius_mm',    min: 1.0,   max: 10.0, step: 0.1,  unit: 'mm' },
      { label: 'Inner radius',    field: 'wehnelt_inner_radius_mm',    min: 0.5,   max: 9.5,  step: 0.1,  unit: 'mm' },
      { label: 'z position',      field: 'wehnelt_z_mm',               min: 1.0,   max: 20.0, step: 0.1,  unit: 'mm' },
      { label: 'Height',          field: 'wehnelt_height_mm',          min: 1.0,   max: 20.0, step: 0.1,  unit: 'mm' },
      { label: 'Cap thickness',   field: 'wehnelt_cap_thickness_mm',   min: 0.1,   max: 2.0,  step: 0.05, unit: 'mm' },
      { label: 'Aperture radius', field: 'wehnelt_aperture_radius_mm', min: 0.1,   max: 3.0,  step: 0.05, unit: 'mm' },
      { label: 'Bias voltage',    field: 'wehnelt_bias_v',             min: -1000, max: 0,    step: 10,   unit: 'V'  },
    ],
  },
  {
    title: 'Anode',
    sliders: [
      { label: 'z position',      field: 'anode_z_mm',               min: 0.5,   max: 15.0, step: 0.1,  unit: 'mm' },
      { label: 'Thickness',       field: 'anode_thickness_mm',       min: 0.1,   max: 5.0,  step: 0.1,  unit: 'mm' },
      { label: 'Outer radius',    field: 'anode_outer_radius_mm',    min: 1.0,   max: 10.0, step: 0.1,  unit: 'mm' },
      { label: 'Aperture radius', field: 'anode_aperture_radius_mm', min: 0.1,   max: 3.0,  step: 0.05, unit: 'mm' },
      { label: 'Voltage',         field: 'anode_voltage_v',          min: -1000, max: 1000,  step: 10,  unit: 'V'  },
    ],
  },
];

function decimalsForStep(step: number): number {
  return Math.max(0, -Math.floor(Math.log10(step)));
}

function formatValue(value: number, step: number): string {
  return value.toFixed(decimalsForStep(step));
}

function buildSliderRow(
  definition: SliderDefinition,
  params: GunParameters,
  onInput: () => void,
  onCommit: () => void,
): HTMLElement {
  const initialValue: number = params[definition.field];

  const row = document.createElement('div');
  row.className = 'slider-row';

  const label = document.createElement('label');
  label.textContent = definition.label;

  const valueSpan = document.createElement('span');
  valueSpan.className = 'slider-value';
  valueSpan.textContent = `${formatValue(initialValue, definition.step)} ${definition.unit}`;

  const input = document.createElement('input');
  input.type = 'range';
  input.min = String(definition.min);
  input.max = String(definition.max);
  input.step = String(definition.step);
  input.value = String(initialValue);

  input.addEventListener('input', () => {
    const value = Number(input.value);
    params[definition.field] = value;
    valueSpan.textContent = `${formatValue(value, definition.step)} ${definition.unit}`;
    onInput();
  });

  // 'change' fires on slider release — trigger an immediate solve instead of
  // waiting for the debounce timer that 'input' events use.
  input.addEventListener('change', () => {
    const value = Number(input.value);
    params[definition.field] = value;
    valueSpan.textContent = `${formatValue(value, definition.step)} ${definition.unit}`;
    onCommit();
  });

  row.appendChild(label);
  row.appendChild(valueSpan);
  row.appendChild(input);
  return row;
}

export function buildControls(
  state: AppState,
  onInput: () => void,
  onCommit: () => void,
): void {
  const panel = document.getElementById('controls');
  if (!panel) throw new Error('#controls element not found');

  for (const group of GROUPS) {
    const section = document.createElement('section');
    section.className = 'electrode-group';

    const heading = document.createElement('h2');
    heading.textContent = group.title;
    section.appendChild(heading);

    for (const def of group.sliders) {
      section.appendChild(buildSliderRow(def, state.parameters, onInput, onCommit));
    }

    panel.appendChild(section);
  }
}
