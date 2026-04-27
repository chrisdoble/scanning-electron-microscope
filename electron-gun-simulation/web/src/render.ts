import type { GunSolution } from './worker-protocol';

/**
 * Renders the electrode mask from `solution` onto `canvas`.
 *
 * Layout (UI.md §6.1):
 *   - Full cross-section: the r ≥ 0 half is mirrored about the z-axis.
 *   - Canvas width  = 2 * n_r − 1  (r = 0 column is the centre; not duplicated).
 *   - Canvas height = n_z.
 *   - i_z = 0 (z = 0) is at the bottom of the canvas (array row 0 → canvas row n_z − 1).
 *
 * Fixed cells are drawn in black; Free cells in white.
 */
export function renderSolution(
  canvas: HTMLCanvasElement,
  solution: GunSolution,
): void {
  const { n_r, n_z, mask } = solution;

  const width = 2 * n_r - 1;
  const height = n_z;

  // Setting width/height clears the canvas and updates its coordinate space.
  canvas.width = width;
  canvas.height = height;

  const ctx = canvas.getContext('2d');
  if (ctx === null) {
    throw new Error('Couldn\'t get visualisation canvas context');
  }

  const imageData = ctx.createImageData(width, height);
  const pixels = imageData.data; // Uint8ClampedArray, RGBA

  // Fill white background.
  for (let i = 0; i < pixels.length; i += 4) {
    pixels[i + 0] = 255; // R
    pixels[i + 1] = 255; // G
    pixels[i + 2] = 255; // B
    pixels[i + 3] = 255; // A
  }

  for (let i_z = 0; i_z < n_z; i_z++) {
    // Flip vertically: i_z = 0 is z = 0 (bottom), but canvas y = 0 is top.
    const canvasY = n_z - 1 - i_z;

    for (let i_r = 0; i_r < n_r; i_r++) {
      if (mask[i_z * n_r + i_r] !== 1) continue;

      // Right half: r = 0 is at the canvas centre (x = n_r − 1).
      setBlack(pixels, canvasY, (n_r - 1) + i_r, width);

      // Left half: mirror, but skip i_r = 0 so the axis column is drawn once.
      if (i_r > 0) {
        setBlack(pixels, canvasY, (n_r - 1) - i_r, width);
      }
    }
  }

  ctx.putImageData(imageData, 0, 0);
}

function setBlack(
  pixels: Uint8ClampedArray,
  y: number,
  x: number,
  width: number,
): void {
  const base = (y * width + x) * 4;
  pixels[base + 0] = 0;
  pixels[base + 1] = 0;
  pixels[base + 2] = 0;
  // Alpha already 255.
}
