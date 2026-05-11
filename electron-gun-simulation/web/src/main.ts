import { buildControls } from './sliders';
import { computeVoltageRange, renderSolution } from './render';
import { initLegendScale, updateLegendLabels } from './legend';
import type { AppState } from './state';
import { DEFAULT_GUN_PARAMETERS, WorkerResponseSchema } from './worker-protocol';

const state: AppState = {
  parameters: { ...DEFAULT_GUN_PARAMETERS },
  pendingParameters: false,
  solution: null,
  solving: false,
};

const canvas = document.querySelector('#visualisation > canvas');
if (!(canvas instanceof HTMLCanvasElement)) {
  throw new Error('Visualisation canvas not found');
}

const legendScale = document.querySelector('#legend-scale');
const legendMax = document.querySelector('#legend-label-max');
const legendMid = document.querySelector('#legend-label-mid');
const legendMin = document.querySelector('#legend-label-min');
if (
  !(legendScale instanceof HTMLCanvasElement) ||
  !(legendMax instanceof HTMLElement) ||
  !(legendMid instanceof HTMLElement) ||
  !(legendMin instanceof HTMLElement)
) {
  throw new Error('Legend elements not found');
}

initLegendScale(legendScale);

const worker = new Worker(new URL('./solver-worker.ts', import.meta.url), { type: 'module' });

worker.addEventListener('message', (event: MessageEvent) => {
  const parseResult = WorkerResponseSchema.safeParse(event.data);
  if (!parseResult.success) {
    console.error('Invalid worker response:', parseResult.error.message);
    alert(`Internal error: invalid worker response — ${parseResult.error.message}`);
    return;
  }
  const msg = parseResult.data;

  if (msg.type === 'success') {
    state.solution = msg.solution;
    const { minV, maxV } = computeVoltageRange(msg.solution);
    renderSolution(canvas, msg.solution, minV, maxV);
    updateLegendLabels(legendMax, legendMid, legendMin, minV, maxV);
    canvas.style.setProperty('--aspect-ratio', String(canvas.width / canvas.height));
    console.log(
      `Solve: ${msg.duration_ms.toFixed(1)} ms` +
      ` (${msg.solution.n_r}×${msg.solution.n_z} grid)`,
    );
  } else {
    console.error('Solve error:', msg.message);
    alert(msg.message);
  }

  state.solving = false;

  if (state.pendingParameters) {
    postToWorker();
  }
});

const PREVIEW_H_SCALE = 4.0;

function postToWorker(): void {
  worker.postMessage({ type: 'solve', parameters: { ...state.parameters } });
  state.solving = true;
  state.pendingParameters = false;
}

function triggerSolve(): void {
  if (!state.solving) {
    postToWorker();
  } else {
    state.pendingParameters = true;
  }
}

// Called on slider 'input' events (fires continuously while dragging).
// Uses a coarser grid so previews complete fast. The pendingParameters
// mechanism ensures a new solve starts immediately after each completes,
// always using the latest parameters — no debounce needed.
function onInput(): void {
  state.parameters.h_scale = PREVIEW_H_SCALE;
  triggerSolve();
}

// Called on slider 'change' (release) — triggers a full-resolution solve.
function onCommit(): void {
  state.parameters.h_scale = 1.0;
  triggerSolve();
}

buildControls(state, onInput, onCommit);

// Start with a fast preview, then queue a full-res solve immediately after.
state.parameters.h_scale = PREVIEW_H_SCALE;
postToWorker();
state.parameters.h_scale = 1.0;
state.pendingParameters = true;
