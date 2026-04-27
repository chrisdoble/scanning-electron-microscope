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
    renderSolution(canvas, msg.solution);
    console.log('Solve result:', {
      n_r: msg.solution.n_r,
      n_z: msg.solution.n_z,
      iterations: msg.solution.iterations,
    });
  } else {
    console.error('Solve error:', msg.message);
    alert(msg.message);
  }

  state.solving = false;

  if (state.pendingParameters) {
    postToWorker();
  }
});

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

let debounceTimer: ReturnType<typeof setTimeout> | null = null;

// Called on slider 'input' events — debounced to avoid flooding the worker.
function onInput(): void {
  if (debounceTimer !== null) clearTimeout(debounceTimer);
  debounceTimer = setTimeout(() => {
    debounceTimer = null;
    triggerSolve();
  }, 500);
}

// Called on slider 'change' (release) — fires immediately, cancels any
// pending debounce so the worker always gets the committed value promptly.
function onCommit(): void {
  if (debounceTimer !== null) {
    clearTimeout(debounceTimer);
    debounceTimer = null;
  }
  triggerSolve();
}

buildControls(state, onInput, onCommit);

// Kick off the initial solve with the default parameters.
postToWorker();
