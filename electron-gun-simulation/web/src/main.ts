import { buildControls } from './sliders';
import { computeVoltageRange, renderSolution } from './render';
import { initLegendScale, updateLegendLabels } from './legend';
import {
  type ViewState,
  computeFitScale,
  clampTranslation,
  applyTransform,
  handleWheel,
  handleDrag,
} from './view';
import type { AppState } from './state';
import { DEFAULT_GUN_PARAMETERS, WorkerResponseSchema } from './worker-protocol';

// Helper: query a required element and assert its type at runtime.
// Returns a non-nullable typed reference that TypeScript carries into closures.
function requireElement<T extends Element>(
  selector: string,
  Ctor: abstract new (...args: any[]) => T,
): T {
  const el = document.querySelector(selector);
  if (!(el instanceof Ctor)) throw new Error(`Required element not found: '${selector}'`);
  return el;
}

const state: AppState = {
  parameters: { ...DEFAULT_GUN_PARAMETERS },
  pendingParameters: false,
  solution: null,
  solving: false,
};

const visualisation = requireElement('#visualisation', HTMLElement);
const canvas         = requireElement('#solution', HTMLCanvasElement);
const overlay        = requireElement('#overlay', HTMLCanvasElement);
const legendScale    = requireElement('#legend-scale', HTMLCanvasElement);
const legendMax      = requireElement('#legend-label-max', HTMLElement);
const legendMid      = requireElement('#legend-label-mid', HTMLElement);
const legendMin      = requireElement('#legend-label-min', HTMLElement);

initLegendScale(legendScale);

// ---- View state ----

let viewState: ViewState = { zoom: 1, translationX: 0, translationY: 0 };
let fitScale = 1;

function updateView(): void {
  const containerW = visualisation.clientWidth;
  const containerH = visualisation.clientHeight;
  viewState = clampTranslation(viewState, fitScale, containerW, containerH, canvas.width, canvas.height);
  applyTransform(canvas, overlay, viewState, fitScale, containerW, containerH);
}

// Recompute fitScale and re-apply transform. Called after each new solve
// (canvas dimensions may have changed) and when the container is resized.
function onViewportChanged(): void {
  if (canvas.width === 0 || canvas.height === 0) return;
  fitScale = computeFitScale(
    visualisation.clientWidth, visualisation.clientHeight,
    canvas.width, canvas.height,
  );
  updateView();
}

new ResizeObserver(onViewportChanged).observe(visualisation);

// ---- Zoom (mouse wheel) ----

visualisation.addEventListener('wheel', (event: WheelEvent) => {
  if (canvas.width === 0) return;
  event.preventDefault();
  const rect = visualisation.getBoundingClientRect();
  const containerW = rect.width;
  const containerH = rect.height;
  viewState = handleWheel(
    event.deltaY,
    event.clientX - rect.left - containerW / 2,
    event.clientY - rect.top  - containerH / 2,
    viewState, fitScale, containerW, containerH, canvas.width, canvas.height,
  );
  applyTransform(canvas, overlay, viewState, fitScale, containerW, containerH);
}, { passive: false });

// ---- Pan (mouse drag) ----

let dragLastX = 0;
let dragLastY = 0;

function onMouseMove(event: MouseEvent): void {
  const dx = event.clientX - dragLastX;
  const dy = event.clientY - dragLastY;
  dragLastX = event.clientX;
  dragLastY = event.clientY;
  viewState = handleDrag(
    dx, dy, viewState, fitScale,
    visualisation.clientWidth, visualisation.clientHeight,
    canvas.width, canvas.height,
  );
  applyTransform(canvas, overlay, viewState, fitScale, visualisation.clientWidth, visualisation.clientHeight);
}

function onMouseUp(): void {
  visualisation.classList.remove('dragging');
  visualisation.removeEventListener('mousemove', onMouseMove);
  window.removeEventListener('mouseup', onMouseUp);
}

visualisation.addEventListener('mousedown', (event: MouseEvent) => {
  dragLastX = event.clientX;
  dragLastY = event.clientY;
  visualisation.classList.add('dragging');
  visualisation.addEventListener('mousemove', onMouseMove);
  window.addEventListener('mouseup', onMouseUp);
});

// ---- Solver worker ----

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
    onViewportChanged();
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
