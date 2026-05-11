import { interpolate } from './turbo_colormap';

/** Draws the turbo colormap into the scale canvas. t=1 (maxV) at top, t=0 (minV) at bottom. */
export function initLegendScale(canvas: HTMLCanvasElement): void {
  const { width, height } = canvas;
  const ctx = canvas.getContext('2d');
  if (ctx === null) throw new Error('Couldn\'t get legend canvas context');
  const imageData = ctx.createImageData(width, height);
  const pixels = imageData.data;
  for (let y = 0; y < height; y++) {
    const t = 1 - y / (height - 1);
    const [r, g, b] = interpolate(t);
    for (let x = 0; x < width; x++) {
      const base = (y * width + x) * 4;
      pixels[base]     = r;
      pixels[base + 1] = g;
      pixels[base + 2] = b;
      pixels[base + 3] = 255;
    }
  }
  ctx.putImageData(imageData, 0, 0);
}

export function updateLegendLabels(
  maxEl: HTMLElement,
  midEl: HTMLElement,
  minEl: HTMLElement,
  minV: number,
  maxV: number,
): void {
  maxEl.textContent = formatVoltage(maxV);
  midEl.textContent = formatVoltage((minV + maxV) / 2);
  minEl.textContent = formatVoltage(minV);
}

function formatVoltage(v: number): string {
  const abs = Math.abs(v);
  if (abs >= 100) return `${v.toFixed(0)} V`;
  if (abs >= 10)  return `${v.toFixed(1)} V`;
  return `${v.toFixed(2)} V`;
}
