// The cold resistance: each setpoint's resistance against the square of its
// current, with error bars, and the fitted line once the fit has run.

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

/** The fitted line, $R = R_0 + b\,I^2$, as plotted. */
export interface PlottedFit {
  /** $R_0$ in ohms, and its combined standard uncertainty. */
  intercept: number;
  interceptUncertainty: number;

  /** $b$ in Ω/A², and its standard uncertainty. */
  slope: number;
  slopeUncertainty: number;

  /** Where the line ends, in A²: at the largest plotted point. */
  end: number;
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

/** The colour of the fitted line and its intercept. */
const FIT_COLOUR = '#d62728';

/** Renders the graph into `container`. */
export function render(container: HTMLElement, characterisation: Characterisation | null): void {
  const points = plottedPoints(characterisation);
  const fit = plottedFit(characterisation, points);

  const graph = Plot.plot({
    document: container.ownerDocument,
    width: 800,
    height: 500,
    marginLeft: 70,
    grid: true,
    caption:
      "Error bars are ±1 standard uncertainty (k = 1). The intercept's is its combined standard uncertainty.",
    x: { domain: X_DOMAIN, label: 'I² (A²)' },
    y: { domain: yDomain(points, fit), label: 'R (Ω)' },
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

      ...(fit === null ? [] : fitMarks(fit)),
    ],
  });

  container.replaceChildren(graph);
}

/**
 * The fitted line from $x = 0$ to its end, and its intercept with the intercept's
 * error bar.
 */
function fitMarks(fit: PlottedFit): Plot.Markish[] {
  const ends = [0, fit.end].map((x) => ({ x, y: fit.intercept + fit.slope * x }));
  const intercept = [{ x: 0, y: fit.intercept }];
  const lower = fit.intercept - fit.interceptUncertainty;
  const upper = fit.intercept + fit.interceptUncertainty;
  // A function, because Plot reads a string channel as a field name, not as
  // text.
  const title = () => fitTooltip(fit);

  return [
    Plot.line(ends, { x: 'x', y: 'y', stroke: FIT_COLOUR, className: 'fit', title, tip: true }),

    Plot.ruleX(intercept, { x: 0, y1: lower, y2: upper, stroke: FIT_COLOUR, className: 'intercept' }),
    ...[lower, upper].map((y) =>
      Plot.ruleY(intercept, {
        y,
        x1: -CAP_WIDTH / 2,
        x2: CAP_WIDTH / 2,
        stroke: FIT_COLOUR,
        className: 'intercept',
      }),
    ),
    Plot.dot(intercept, {
      x: 'x',
      y: 'y',
      symbol: 'diamond',
      r: 5,
      fill: FIT_COLOUR,
      className: 'intercept',
      title,
      tip: true,
    }),
  ];
}

/**
 * The fitted line, once the fit has run, or `null` before.
 *
 * It ends at the largest plotted point. Every point has been analysed by the
 * time the fit runs, so there's always one.
 */
export function plottedFit(
  characterisation: Characterisation | null,
  points: PlottedPoint[],
): PlottedFit | null {
  const analysis = characterisation?.cold_resistance?.analysis;
  if (analysis == null) {
    return null;
  }

  return {
    intercept: analysis.resistance_ohms.value,
    interceptUncertainty: analysis.resistance_ohms.uncertainty,
    slope: analysis.slope_ohms_per_amp_squared.value,
    slopeUncertainty: analysis.slope_ohms_per_amp_squared.uncertainty,
    end: Math.max(...points.map((point) => point.x), 0),
  };
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
 * The $y$ axis's range in ohms: from 1% below the bottom of the lowest error
 * bar to 1% above the top of the highest. Fixed at `EMPTY_Y_DOMAIN` while
 * there are no points.
 *
 * Once there's a fit, its intercept's error bar and the line's end count too,
 * so they're always in view. With a positive slope the intercept sits below
 * every point.
 *
 * Fitted to the points rather than fixed, because on a fixed range wide enough
 * for every filament and the 1 Ω bench-test resistor, a 0.1 Ω filament's
 * points would sit along the bottom with error bars far too small to see.
 */
export function yDomain(points: PlottedPoint[], fit: PlottedFit | null): [number, number] {
  if (points.length === 0) {
    return EMPTY_Y_DOMAIN;
  }

  const bottoms = points.map((point) => point.y - point.yUncertainty);
  const tops = points.map((point) => point.y + point.yUncertainty);
  if (fit !== null) {
    const end = fit.intercept + fit.slope * fit.end;
    bottoms.push(fit.intercept - fit.interceptUncertainty, end);
    tops.push(fit.intercept + fit.interceptUncertainty, end);
  }

  const lowest = Math.min(...bottoms);
  const highest = Math.max(...tops);
  return [lowest * 0.99, highest * 1.01];
}

/** The fit's tooltip, on the line and the intercept. */
function fitTooltip(fit: PlottedFit): string {
  return [
    'Fit: R = R₀ + b I²',
    `R₀ = ${withUncertainty(fit.intercept, fit.interceptUncertainty)} Ω (combined)`,
    `b = ${withUncertainty(fit.slope, fit.slopeUncertainty)} Ω/A²`,
  ].join('\n');
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
