// The cold resistance: each setpoint's resistance against the square of its
// current, with error bars.

import * as Plot from '@observablehq/plot';
import type { Characterisation } from '../generated/characterisation.ts';

/** One setpoint, as plotted. */
export interface PlottedPoint {
  /** The setpoint in amperes. */
  setpoint: number;

  /** The square of the current in A², and its standard uncertainty. */
  x: number;
  xUncertainty: number;

  /** The resistance in ohms, and its standard uncertainty. */
  y: number;
  yUncertainty: number;
}

/**
 * The $x$ axis's range in A². Fixed rather than fitted to the points, so they
 * appear in place as they're measured: the largest setpoint is 300 mA, which
 * is 0.09 A².
 */
const X_DOMAIN: [number, number] = [0, 0.095];

/** The $y$ axis's range in ohms before there are any points. */
const EMPTY_Y_DOMAIN: [number, number] = [0, 2];

/**
 * How wide the caps on the error bars are, in A². About 6 pixels at the
 * graph's width.
 */
const CAP_WIDTH = 0.0008;

/** Renders the graph into `container`. */
export function render(container: HTMLElement, characterisation: Characterisation | null): void {
  const points = plottedPoints(characterisation);

  const graph = Plot.plot({
    document: container.ownerDocument,
    width: 800,
    height: 500,
    marginLeft: 70,
    grid: true,
    caption: 'Error bars are ±1 standard uncertainty (k = 1).',
    x: { domain: X_DOMAIN, label: 'I² (A²)' },
    y: { domain: yDomain(points), label: 'R (Ω)' },
    marks: [
      // Vertical error bars, with caps.
      Plot.ruleX(points, {
        x: 'x',
        y1: (point: PlottedPoint) => point.y - point.yUncertainty,
        y2: (point: PlottedPoint) => point.y + point.yUncertainty,
      }),
      ...[-1, 1].map((sign) =>
        Plot.ruleY(points, {
          y: (point: PlottedPoint) => point.y + sign * point.yUncertainty,
          x1: (point: PlottedPoint) => point.x - CAP_WIDTH / 2,
          x2: (point: PlottedPoint) => point.x + CAP_WIDTH / 2,
        }),
      ),

      // Horizontal error bars. Usually too small to see, but they're recorded.
      Plot.ruleY(points, {
        y: 'y',
        x1: (point: PlottedPoint) => point.x - point.xUncertainty,
        x2: (point: PlottedPoint) => point.x + point.xUncertainty,
      }),

      Plot.dot(points, {
        x: 'x',
        y: 'y',
        r: 3.5,
        fill: 'currentColor',
        title: tooltip,
        tip: true,
      }),
    ],
  });

  container.replaceChildren(graph);
}

/**
 * The points to plot: every setpoint whose analysis has run. A setpoint
 * without one has no resistance yet. That's brief, between the point being
 * saved and its analysis finishing.
 */
export function plottedPoints(characterisation: Characterisation | null): PlottedPoint[] {
  return (characterisation?.cold_resistance?.points ?? []).flatMap((point) =>
    point.analysis == null
      ? []
      : [
          {
            setpoint: point.setpoint_amps,
            x: point.analysis.current_squared_amps_squared.value,
            xUncertainty: point.analysis.current_squared_amps_squared.uncertainty,
            y: point.analysis.resistance_ohms.value,
            yUncertainty: point.analysis.resistance_ohms.uncertainty,
          },
        ],
  );
}

/**
 * The $y$ axis's range in ohms: from 10% below the bottom of the lowest error
 * bar to 10% above the top of the highest. Fixed at `EMPTY_Y_DOMAIN` while
 * there are no points.
 *
 * Fitted to the points rather than fixed, because on a fixed range wide enough
 * for every filament and the 1 Ω bench-test resistor, a 0.1 Ω filament's
 * points would sit along the bottom with error bars far too small to see.
 */
export function yDomain(points: PlottedPoint[]): [number, number] {
  if (points.length === 0) {
    return EMPTY_Y_DOMAIN;
  }
  const lowest = Math.min(...points.map((point) => point.y - point.yUncertainty));
  const highest = Math.max(...points.map((point) => point.y + point.yUncertainty));
  return [lowest * 0.9, highest * 1.1];
}

/** A point's tooltip. */
function tooltip(point: PlottedPoint): string {
  return [
    `Setpoint: ${(point.setpoint * 1000).toFixed(0)} mA`,
    `R = ${withUncertainty(point.y, point.yUncertainty)} Ω`,
    `I² = ${withUncertainty(point.x, point.xUncertainty)} A²`,
  ].join('\n');
}

/**
 * `value ± uncertainty`, with the uncertainty to two significant figures and
 * the value to the same number of decimal places.
 */
export function withUncertainty(value: number, uncertainty: number): string {
  if (!(uncertainty > 0)) {
    return `${value} ± ${uncertainty}`;
  }
  const decimals = Math.max(0, 1 - Math.floor(Math.log10(uncertainty)));
  return `${value.toFixed(decimals)} ± ${uncertainty.toFixed(decimals)}`;
}
