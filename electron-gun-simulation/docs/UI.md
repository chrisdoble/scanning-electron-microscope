# UI

This document specifies the web application's architecture: layout, state management, how slider changes trigger solves, and how results are visualised. It is the authoritative reference for anything in `web/`. See `ARCHITECTURE.md` for how the web app fits into the overall project and how it communicates with the wasm layer.

## 1. Technology choices

- **Framework:** None (vanilla TypeScript). The UI is simple enough — sliders, a canvas, and some text — that a framework would add dependency weight without meaningful benefit. If this changes, React is the fallback.
- **Rendering:** HTML5 Canvas 2D for the field visualisation. WebGL is unnecessary for a 2D colourmap on a grid of this size.
- **Styling:** Plain CSS. No preprocessor or utility framework.
- **Package manager:** pnpm.
- **Build tool:** Vite.

## 2. Page layout

The page has two main regions side by side (on wide screens) or stacked (on narrow screens):

1. **Controls panel** (left / top) — sliders for all `GunParameters` fields, grouped by electrode. Displays current values with units.
2. **Visualisation panel** (right / bottom) — a single Canvas element showing the potential field as a colourmap with the electrode geometry overlaid, rendered as a full cross-section (mirrored about the z-axis, see §6.1).

The visualisation panel should fill available space and maintain the correct aspect ratio of the rendered cross-section.

## 3. State management

All application state lives in a single plain TypeScript object:

```typescript
interface AppState {
  params: GunParameters;         // current slider values, in mm and V
  solution: GunSolution | null;  // most recent solve result, or null if not yet solved
  solving: boolean;              // true while a solve is in progress
  pendingParams: boolean;        // true if params changed while a solve was in progress
}
```

There is no state management library. State updates follow a simple pattern: mutate the state, then call a `render()` function that reads the state and updates the DOM and canvas.

## 4. Slider configuration

Each field in `GunParameters` has a corresponding slider. Sliders are grouped by electrode:

**Filament:**
- Radius (mm)
- Thickness (mm)
- z position (mm)
- Voltage (V)

**Wehnelt cylinder:**
- Outer radius (mm)
- Inner radius (mm)
- z position (mm)
- Height (mm)
- Cap thickness (mm)
- Aperture radius (mm)
- Bias voltage (V, relative to filament)

**Anode:**
- z position (mm)
- Thickness (mm)
- Outer radius (mm)
- Aperture radius (mm)
- Voltage (V)

Each slider displays its current numeric value next to it, with the unit.

## 5. Solve lifecycle

### 5.1 Web Worker

The solver runs in a Web Worker so it does not block the main thread. The worker:

1. Imports and initialises the wasm module (`await init()`).
2. Listens for messages containing `GunParameters`.
3. Calls `solve_electron_gun(params)`.
4. Posts the result (or error) back to the main thread.

The wasm module must be instantiated inside the worker — it cannot be shared from the main thread. The worker is created once on page load and reused for all subsequent solves.

### 5.2 Triggering a solve

When any slider value changes:

1. Update `params` in the app state.
2. If `solving` is false (the worker is idle): post `params` to the worker, set `solving = true`, set `pendingParams = false`.
3. If `solving` is true (the worker is busy): set `pendingParams = true`. Do not post to the worker. The latest params will be sent when the current solve finishes (see §5.3).

This ensures at most one solve is in flight at a time, and the worker always solves the most recent params rather than working through a backlog of stale intermediate values.

### 5.3 Receiving results

When the worker posts a result back:

1. If it's a success: store the `GunSolution` in state, call `render()`.
2. If it's an error: log the error with `console.error`, call `alert()` with the error message.
3. Set `solving = false`.
4. If `pendingParams` is true: immediately post the current `params` to the worker, set `solving = true`, set `pendingParams = false`.

### 5.4 Debouncing

Apply a debounce of ~100-200ms on slider `input` events before triggering the logic in §5.2. This prevents triggering on every pixel of slider movement. Use the slider's `change` event (fires on release) as an immediate trigger with no debounce, so releasing the slider always triggers promptly.

## 6. Visualisation

### 6.1 Full cross-section rendering

The simulation grid only contains r ≥ 0 (the right half of the cross-section), since the field is axisymmetric. To show the full cross-section as if the gun were sliced along the z-axis, the canvas renders the grid mirrored about the z-axis:

- The z-axis runs vertically down the centre of the canvas.
- To the right of centre: the grid as stored, with r = 0 at the centre and r = R_max at the right edge.
- To the left of centre: the same grid horizontally flipped, with r = R_max at the left edge.
- r = 0 is drawn once (the centre column), not duplicated.

The canvas width in grid units is `2 * (N_r - 1) + 1` pixels (or scaled). The canvas height in grid units is `N_z` pixels (or scaled).

z = 0 is at the bottom of the canvas, z = Z_max at the top (vertical flip from the array's memory order — see PHYSICS.md §2.1). This combined with the horizontal mirroring means the visualisation shows the gun as a vertical cross-section: the filament and Wehnelt cup are at the top, the anode is below, and the beam axis runs down the centre.

### 6.2 Potential field colourmap

Render the potential grid as a colourmap:

- Each grid point maps to a pixel (or small rectangle if the canvas is larger than the grid).
- Use the **Turbo** colourmap (developed by Anton Mikhailov at Google). Turbo is a sequential, perceptually near-uniform, full-spectrum colourmap that shows fine gradient detail across the entire voltage range. This is preferable to a diverging colourmap (like coolwarm) for this application because the potentials span a continuous range from a large negative value (filament) to ground (anode) — a diverging map would waste most of its dynamic range on one side.
- The colour scale should span the full voltage range of the current solution (min V to max V).

### 6.3 Electrode overlay

Draw the electrode boundaries on top of the colourmap using the mask data from `GunSolution`. Points where `mask[i] === 1` (Fixed) are drawn in a solid colour (e.g. dark grey or black) so the electrode shapes are clearly visible against the field. The mirroring from §6.1 applies to the mask in the same way as the potential grid.

### 6.4 Colour bar

Display a colour bar alongside the canvas showing the voltage-to-colour mapping, with labelled tick marks at key voltages (filament, Wehnelt, anode, 0 V).

## 7. Wasm initialisation

On page load:

1. Create the Web Worker. The worker initialises the wasm module internally (`await init()`).
2. Set initial slider values to defaults and trigger an initial solve by posting the default params to the worker. The worker will process the message once wasm initialisation is complete.

## 8. Error handling

Errors from the wasm layer (geometry validation failures, solver divergence) are logged with `console.error` and displayed to the user via `alert()`. The previous visualisation remains on screen (if any) so the user can see what they had before the error and adjust sliders to fix it.
