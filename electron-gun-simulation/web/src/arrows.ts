import type { GunSolution } from './worker-protocol';
import { type ViewState, containerToSolution } from './view';

const SPACING = 50;           // container pixels between arrow centres
const MAX_LEN = SPACING * 0.75;
const MIN_LEN = SPACING * 0.20;
const HEAD_ANGLE = Math.PI / 6; // 30° arrowhead half-angle

export function renderArrows(
  overlay: HTMLCanvasElement,
  solution: GunSolution,
  viewState: ViewState,
  fitScale: number,
  containerW: number,
  containerH: number,
  flip: boolean,
): void {
  const ctx = overlay.getContext('2d');
  if (ctx === null) return;

  const { n_r, n_z, e_r_v_per_m, e_z_v_per_m, mask } = solution;
  const canvasW = 2 * n_r - 1;
  const canvasH = n_z;

  type Arrow = {
    cx: number; cy: number;   // centre in container space
    dx: number; dy: number;   // unit direction in container space
    mag: number;
  };

  // --- Step 1–4: collect valid arrow positions and sample field ---

  const arrows: Arrow[] = [];
  let maxMag = 0;
  let minMag = Infinity;

  for (let cy = SPACING / 2; cy < containerH; cy += SPACING) {
    for (let cx = SPACING / 2; cx < containerW; cx += SPACING) {
      // Convert container point to solution canvas pixel coordinates.
      const { x: canvasX, y: canvasY } = containerToSolution(
        cx, cy, viewState, fitScale, containerW, containerH, canvasW, canvasH,
      );

      // Skip points outside the solution canvas (over the black background).
      if (canvasX < 0 || canvasX > canvasW - 1 || canvasY < 0 || canvasY > canvasH - 1) continue;

      // Determine which half of the mirrored canvas this point is on.
      // Right half: i_r increases with canvas x. Left half: reflected.
      const isRightHalf = canvasX >= n_r - 1;
      const sign = isRightHalf ? 1 : -1;
      const i_r = Math.round(isRightHalf ? canvasX - (n_r - 1) : (n_r - 1) - canvasX);
      const i_z = Math.round((n_z - 1) - canvasY);

      if (i_r < 0 || i_r >= n_r || i_z < 0 || i_z >= n_z) continue;

      const k = i_z * n_r + i_r;

      // Skip electrode cells.
      if (mask[k] === 1) continue;

      const er = e_r_v_per_m[k];
      const ez = e_z_v_per_m[k];
      const mag = Math.sqrt(er * er + ez * ez);

      // Skip zero-field points (no direction to draw).
      if (mag === 0) continue;

      if (mag > maxMag) maxMag = mag;
      if (mag < minMag) minMag = mag;

      // Map field direction to container space:
      //   E_r → x, flipped for the left half (E_r is outward from the axis on both sides)
      //   E_z → -y (canvas y is downward; z is upward)
      arrows.push({ cx, cy, dx: (sign * er) / mag, dy: -ez / mag, mag });
    }
  }

  if (arrows.length === 0 || maxMag === 0) return;

  // --- Steps 5–9: draw length-scaled arrows ---

  ctx.strokeStyle = 'black';
  ctx.lineWidth = 1;

  const magRange = maxMag === minMag ? 1 : maxMag - minMag;

  for (const { cx, cy, dx, dy, mag } of arrows) {
    const t = (mag - minMag) / magRange;
    const length = MIN_LEN + t * (MAX_LEN - MIN_LEN);
    const halfLen = length / 2;

    const drawDx = flip ? -dx : dx;
    const drawDy = flip ? -dy : dy;

    const tailX = cx - drawDx * halfLen;
    const tailY = cy - drawDy * halfLen;
    const tipX  = cx + drawDx * halfLen;
    const tipY  = cy + drawDy * halfLen;

    const headLen = Math.min(length * 0.3, 6);
    const angle = Math.atan2(drawDy, drawDx);

    ctx.beginPath();
    ctx.moveTo(tailX, tailY);
    ctx.lineTo(tipX, tipY);
    ctx.moveTo(tipX, tipY);
    ctx.lineTo(tipX - headLen * Math.cos(angle - HEAD_ANGLE), tipY - headLen * Math.sin(angle - HEAD_ANGLE));
    ctx.moveTo(tipX, tipY);
    ctx.lineTo(tipX - headLen * Math.cos(angle + HEAD_ANGLE), tipY - headLen * Math.sin(angle + HEAD_ANGLE));
    ctx.stroke();
  }
}
