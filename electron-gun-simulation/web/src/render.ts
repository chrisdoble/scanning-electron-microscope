import type { GunSolution } from './worker-protocol';
import { interpolate } from './turbo_colormap';

/**
 * Renders the potential field from `solution` onto `canvas` using the Turbo
 * colormap. Electrode cells (Fixed) are drawn black.
 *
 * Layout (UI.md §6.1):
 *   - Full cross-section: the r ≥ 0 half is mirrored about the z-axis.
 *   - Canvas width  = 2 * n_r − 1  (r = 0 column is the centre; not duplicated).
 *   - Canvas height = n_z.
 *   - i_z = 0 (z = 0) is at the bottom of the canvas (array row 0 → canvas row n_z − 1).
 *
 * The potential range is mapped linearly to [0, 1] across the full colormap.
 */
export function renderSolution(
  canvas: HTMLCanvasElement,
  solution: GunSolution,
): void {
  const { n_r, n_z, mask, potential_v } = solution;

  const width = 2 * n_r - 1;
  const height = n_z;

  canvas.width = width;
  canvas.height = height;

  const ctx = canvas.getContext('2d');
  if (ctx === null) {
    throw new Error('Couldn\'t get visualisation canvas context');
  }

  // Find potential range across all cells for colormap normalisation.
  let minV = Infinity;
  let maxV = -Infinity;
  for (let i = 0; i < potential_v.length; i++) {
    const v = potential_v[i];
    if (v < minV) minV = v;
    if (v > maxV) maxV = v;
  }
  const range = maxV - minV;

  const imageData = ctx.createImageData(width, height);
  const pixels = imageData.data; // Uint8ClampedArray, RGBA

  for (let i_z = 0; i_z < n_z; i_z++) {
    const canvasY = n_z - 1 - i_z; // flip: i_z=0 is bottom, canvas y=0 is top

    for (let i_r = 0; i_r < n_r; i_r++) {
      const idx = i_z * n_r + i_r;
      const isFixed = mask[idx] === 1;

      let r: number, g: number, b: number;
      if (isFixed) {
        r = g = b = 0; // electrodes: black
      } else {
        const t = range > 0 ? (potential_v[idx] - minV) / range : 0;
        [r, g, b] = interpolate(t);
      }

      // Right half: r = 0 is at canvas centre (x = n_r − 1).
      setPixel(pixels, canvasY, (n_r - 1) + i_r, width, r, g, b);

      // Left half: mirror, skip i_r = 0 so axis column is drawn once.
      if (i_r > 0) {
        setPixel(pixels, canvasY, (n_r - 1) - i_r, width, r, g, b);
      }
    }
  }

  ctx.putImageData(imageData, 0, 0);
  drawLegend(ctx, minV, maxV);
}

const LEGEND_X = 9.5;
const LEGEND_Y = 9.5;
const LEGEND_W = 300;
const LEGEND_H = 30;

function drawLegend(ctx: CanvasRenderingContext2D, minV: number, maxV: number): void {
  const gradient = ctx.createLinearGradient(LEGEND_X, 0, LEGEND_X + LEGEND_W, 0);
  const colorStopCount = 20;
  for (let i = 0; i <= colorStopCount; i++) {
    const t = i / colorStopCount;
    const [r, g, b] = interpolate(t);
    gradient.addColorStop(t, `rgb(${r},${g},${b})`);
  }
  ctx.fillStyle = gradient;
  ctx.fillRect(LEGEND_X, LEGEND_Y, LEGEND_W, LEGEND_H);

  ctx.strokeStyle = 'black';
  ctx.lineWidth = 1;
  ctx.strokeRect(LEGEND_X, LEGEND_Y, LEGEND_W, LEGEND_H);

  ctx.fillStyle = 'white';
  ctx.font = '14px monospace';
  ctx.textBaseline = 'top';
  const labelY = LEGEND_Y + LEGEND_H + 3;

  ctx.textAlign = 'left';
  ctx.fillText(formatVoltage(minV), LEGEND_X, labelY);

  ctx.textAlign = 'center';
  ctx.fillText(formatVoltage((minV + maxV) / 2), LEGEND_X + LEGEND_W / 2, labelY);

  ctx.textAlign = 'right';
  ctx.fillText(formatVoltage(maxV), LEGEND_X + LEGEND_W, labelY);
}

function formatVoltage(v: number): string {
  const abs = Math.abs(v);
  if (abs >= 100) return `${v.toFixed(0)} V`;
  if (abs >= 10) return `${v.toFixed(1)} V`;
  return `${v.toFixed(2)} V`;
}

function setPixel(
  pixels: Uint8ClampedArray,
  y: number,
  x: number,
  width: number,
  r: number,
  g: number,
  b: number,
): void {
  const base = (y * width + x) * 4;
  pixels[base + 0] = r;
  pixels[base + 1] = g;
  pixels[base + 2] = b;
  pixels[base + 3] = 255;
}
