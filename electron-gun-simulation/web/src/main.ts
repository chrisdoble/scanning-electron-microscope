import { buildControls } from './sliders';
import { renderSolution } from './render';
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
    renderSolution(canvas, msg.solution, !isPreview);
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

let isPreview = false;

function postToWorker(): void {
  state.parameters.h_scale = isPreview ? PREVIEW_H_SCALE : 1.0;
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
  isPreview = true;
  triggerSolve();
}

// Called on slider 'change' (release) — triggers a full-resolution solve.
function onCommit(): void {
  isPreview = false;
  triggerSolve();
}

buildControls(state, onInput, onCommit);

// Kick off the initial solve with the default parameters.
postToWorker();
