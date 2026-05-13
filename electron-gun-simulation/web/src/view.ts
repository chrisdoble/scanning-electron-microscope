export interface ViewState {
  zoom: number;
  translationX: number;
  translationY: number;
}

export const MAX_ZOOM = 16;
const MIN_ZOOM = 1;
const ZOOM_FACTOR = 1.1; // per wheel tick

export function computeFitScale(
  containerW: number, containerH: number,
  canvasW: number, canvasH: number,
): number {
  return Math.min(containerW / canvasW, containerH / canvasH);
}

export function clampTranslation(
  state: ViewState,
  fitScale: number,
  containerW: number, containerH: number,
  canvasW: number, canvasH: number,
): ViewState {
  const totalScale = fitScale * state.zoom;
  const maxTx = Math.max(0, (canvasW * totalScale - containerW) / 2);
  const maxTy = Math.max(0, (canvasH * totalScale - containerH) / 2);
  return {
    ...state,
    translationX: Math.max(-maxTx, Math.min(maxTx, state.translationX)),
    translationY: Math.max(-maxTy, Math.min(maxTy, state.translationY)),
  };
}

// Sets the CSS transform on the solution canvas and redraws the overlay.
// The solution canvas is assumed to be flexbox-centred inside the container
// (its centre is at the container's centre before any transform).
export function applyTransform(
  solutionCanvas: HTMLCanvasElement,
  overlayCanvas: HTMLCanvasElement,
  state: ViewState,
  fitScale: number,
  containerW: number,
  containerH: number,
): void {
  const totalScale = fitScale * state.zoom;
  solutionCanvas.style.transform =
    `translate(${state.translationX}px, ${state.translationY}px) scale(${totalScale})`;

  overlayCanvas.width = containerW;
  overlayCanvas.height = containerH;
  // Clear overlay — electric field arrows will be drawn here in future.
  overlayCanvas.getContext('2d')?.clearRect(0, 0, containerW, containerH);
}

// Updates ViewState for a wheel zoom gesture, keeping the canvas pixel
// under the cursor stationary. cursorX/Y are relative to the container centre.
export function handleWheel(
  deltaY: number,
  cursorX: number, cursorY: number,
  state: ViewState,
  fitScale: number,
  containerW: number, containerH: number,
  canvasW: number, canvasH: number,
): ViewState {
  const factor = deltaY < 0 ? ZOOM_FACTOR : 1 / ZOOM_FACTOR;
  const newZoom = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, state.zoom * factor));
  const ratio = newZoom / state.zoom;
  return clampTranslation(
    {
      zoom: newZoom,
      translationX: cursorX - (cursorX - state.translationX) * ratio,
      translationY: cursorY - (cursorY - state.translationY) * ratio,
    },
    fitScale, containerW, containerH, canvasW, canvasH,
  );
}

// Updates ViewState for a drag gesture. dx/dy are in CSS pixels.
export function handleDrag(
  dx: number, dy: number,
  state: ViewState,
  fitScale: number,
  containerW: number, containerH: number,
  canvasW: number, canvasH: number,
): ViewState {
  return clampTranslation(
    { ...state, translationX: state.translationX + dx, translationY: state.translationY + dy },
    fitScale, containerW, containerH, canvasW, canvasH,
  );
}

// Maps a point in solution canvas pixel space (0,0 = top-left of canvas buffer)
// to container space (relative to container top-left). Inverse of containerToSolution.
export function solutionToContainer(
  x: number, y: number,
  state: ViewState, fitScale: number,
  containerW: number, containerH: number,
  canvasW: number, canvasH: number,
): { cx: number; cy: number } {
  const totalScale = fitScale * state.zoom;
  return {
    cx: (x - canvasW / 2) * totalScale + containerW / 2 + state.translationX,
    cy: (y - canvasH / 2) * totalScale + containerH / 2 + state.translationY,
  };
}

// Maps a point in container space (relative to container top-left) to
// solution canvas pixel space (0,0 = top-left of canvas buffer).
export function containerToSolution(
  cx: number, cy: number,
  state: ViewState, fitScale: number,
  containerW: number, containerH: number,
  canvasW: number, canvasH: number,
): { x: number; y: number } {
  const totalScale = fitScale * state.zoom;
  return {
    x: (cx - containerW / 2 - state.translationX) / totalScale + canvasW / 2,
    y: (cy - containerH / 2 - state.translationY) / totalScale + canvasH / 2,
  };
}
