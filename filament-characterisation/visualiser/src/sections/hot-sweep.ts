// The hot sweep: each point's resistance against the square of its current,
// and against its power, going up and coming back down, with error bars. The
// difference between up and down at a setpoint shows how much the filament's
// mount warmed in between.

import * as Plot from '@observablehq/plot';
import { caption, withUncertainty } from '../shared.ts';
import type { Characterisation, HotSweepDirection } from '../generated/characterisation.ts';

/** One point of the sweep, as plotted. */
export interface SweepPoint {
  /** The current the point was measured at. */
  setpoint_amps: number;

  direction: HotSweepDirection;

  /** The square of the measured current. */
  current_squared_amps_squared: number;

  /** The power, VI. */
  power_milliwatts: number;

  /** The resistance, and its statistical standard uncertainty. */
  resistance_ohms: number;
  resistance_uncertainty_ohms: number;

  /** The oscilloscope's vertical scale. */
  vertical_scale_volts_per_division: number;
}

/** The directions, in the order their colours and symbols are assigned. */
const DIRECTIONS: HotSweepDirection[] = ['Up', 'Down'];

/** Up and down's colours: a pair that stays distinct with colour blindness. */
const DIRECTION_COLOURS = ['#0072b2', '#e69f00'];

/** Up and down's marker shapes, so they're distinct without colour too. */
const DIRECTION_SYMBOLS = ['circle', 'triangle'];

/**
 * How far the I² axis runs past the maximum current squared, as a fraction,
 * so a point at the maximum isn't on the edge.
 */
const X_MARGIN = 0.05;

/** The I² axis's range in A² before the sweep has started. */
const EMPTY_CURRENT_SQUARED_DOMAIN: [number, number] = [0, 1];

/** The power axis's range in milliwatts before there are any points. */
const EMPTY_POWER_DOMAIN: [number, number] = [0, 100];

/** The resistance axis's range in ohms before there are any points. */
const EMPTY_RESISTANCE_DOMAIN: [number, number] = [0, 2];

/** How wide the caps on the error bars are, as a fraction of the x axis. */
const CAP_FRACTION = 0.008;

/** What the error bars cover, shown once beneath both graphs. */
const CAPTION =
  "Error bars are ±1 statistical standard uncertainty (k = 1). They don't include the oscilloscope's gain error or the supply's readback errors, which are shared between points: they don't affect the difference between up and down at a setpoint, but each absolute value carries a few percent.";

/** Renders the two graphs, and their shared caption, into `container`. */
export function render(container: HTMLElement, characterisation: Characterisation | null): void {
  const document = container.ownerDocument;
  const points = sweepPoints(characterisation);
  const resistance = resistanceDomain(points);

  container.replaceChildren(
    graph(document, points, {
      title: 'Resistance against current squared',
      x: (point) => point.current_squared_amps_squared,
      domain: currentSquaredDomain(characterisation),
      label: 'I² (A²)',
      resistance,
    }),
    graph(document, points, {
      title: 'Resistance against power',
      x: (point) => point.power_milliwatts,
      domain: powerDomain(points),
      label: 'P (mW)',
      resistance,
    }),
    caption(document, CAPTION),
  );
}

/** One graph of resistance against `x`. */
function graph(
  document: Document,
  points: SweepPoint[],
  axis: {
    title: string;
    x: (point: SweepPoint) => number;
    domain: [number, number];
    label: string;
    resistance: [number, number];
  },
): SVGSVGElement | HTMLElement {
  const cap = (axis.domain[1] - axis.domain[0]) * CAP_FRACTION;

  return Plot.plot({
    document,
    width: 800,
    height: 500,
    marginLeft: 70,
    grid: true,
    title: axis.title,
    x: { domain: axis.domain, label: axis.label },
    y: { domain: axis.resistance, label: 'R (Ω)' },
    color: { domain: DIRECTIONS, range: DIRECTION_COLOURS },
    symbol: { domain: DIRECTIONS, range: DIRECTION_SYMBOLS, legend: true },
    marks: [
      // Vertical error bars, with caps.
      Plot.ruleX(points, {
        x: axis.x,
        y1: (point: SweepPoint) => point.resistance_ohms - point.resistance_uncertainty_ohms,
        y2: (point: SweepPoint) => point.resistance_ohms + point.resistance_uncertainty_ohms,
        stroke: 'direction',
      }),
      ...[-1, 1].map((sign) =>
        Plot.ruleY(points, {
          y: (point: SweepPoint) => point.resistance_ohms + sign * point.resistance_uncertainty_ohms,
          x1: (point: SweepPoint) => axis.x(point) - cap / 2,
          x2: (point: SweepPoint) => axis.x(point) + cap / 2,
          stroke: 'direction',
        }),
      ),

      // A mark per direction, so each can be found by its class.
      ...DIRECTIONS.map((direction) =>
        Plot.dot(
          points.filter((point) => point.direction === direction),
          {
            x: axis.x,
            y: 'resistance_ohms',
            r: 4,
            fill: 'direction',
            symbol: 'direction',
            className: direction.toLowerCase(),
            title: tooltip,
            tip: true,
          },
        ),
      ),
    ],
  });
}

/**
 * The I² axis's range: from 0 to a little past the maximum current squared, so
 * points appear in place as they're measured. Fixed at
 * `EMPTY_CURRENT_SQUARED_DOMAIN` before the sweep has started.
 */
export function currentSquaredDomain(characterisation: Characterisation | null): [number, number] {
  const maximum = characterisation?.hot_sweep?.maximum_current_amps;
  if (maximum == null) {
    return EMPTY_CURRENT_SQUARED_DOMAIN;
  }
  return [0, maximum * maximum * (1 + X_MARGIN)];
}

/**
 * The power axis's range in milliwatts: from 0 to a little past the largest
 * point's. The maximum power isn't known in advance, since it depends on the
 * resistance.
 */
export function powerDomain(points: SweepPoint[]): [number, number] {
  if (points.length === 0) {
    return EMPTY_POWER_DOMAIN;
  }
  return [0, Math.max(...points.map((point) => point.power_milliwatts)) * (1 + X_MARGIN)];
}

/**
 * The resistance axis's range in ohms: from 1% below the bottom of the lowest
 * error bar to 1% above the top of the highest, as the cold resistance's is.
 * Shared by both graphs, so they can be compared.
 */
export function resistanceDomain(points: SweepPoint[]): [number, number] {
  if (points.length === 0) {
    return EMPTY_RESISTANCE_DOMAIN;
  }
  const lowest = Math.min(
    ...points.map((point) => point.resistance_ohms - point.resistance_uncertainty_ohms),
  );
  const highest = Math.max(
    ...points.map((point) => point.resistance_ohms + point.resistance_uncertainty_ohms),
  );
  return [lowest * 0.99, highest * 1.01];
}

/**
 * The points to plot: every point whose analysis has run. A point without one
 * has no resistance yet. That's brief, between the point being saved and its
 * analysis finishing.
 */
export function sweepPoints(characterisation: Characterisation | null): SweepPoint[] {
  return (characterisation?.hot_sweep?.points ?? []).flatMap((point) => {
    const analysis = point.analysis;
    if (analysis == null) {
      return [];
    }
    return [
      {
        setpoint_amps: point.setpoint_amps,
        direction: point.direction,
        current_squared_amps_squared: analysis.current_amps.value ** 2,
        power_milliwatts: analysis.voltage_volts.value * analysis.current_amps.value * 1000,
        resistance_ohms: analysis.resistance_ohms.value,
        resistance_uncertainty_ohms: analysis.resistance_ohms.uncertainty,
        vertical_scale_volts_per_division: point.vertical_scale_volts_per_division,
      },
    ];
  });
}

/** A point's tooltip. */
function tooltip(point: SweepPoint): string {
  return [
    `Setpoint: ${(point.setpoint_amps * 1000).toFixed(0)} mA (${point.direction.toLowerCase()})`,
    `R = ${withUncertainty(point.resistance_ohms, point.resistance_uncertainty_ohms)} Ω`,
    `P = ${point.power_milliwatts.toFixed(1)} mW`,
    `Scale: ${scaleLabel(point.vertical_scale_volts_per_division)}`,
  ].join('\n');
}

/** A vertical scale for display, e.g. "20 mV/div" or "2 V/div". */
function scaleLabel(scale: number): string {
  return scale < 1 ? `${(scale * 1000).toFixed(0)} mV/div` : `${scale} V/div`;
}
