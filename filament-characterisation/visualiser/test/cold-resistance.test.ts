// @vitest-environment jsdom

import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { parse } from '../src/data.ts';
import type { Characterisation } from '../src/generated/characterisation.ts';
import {
  plottedFit,
  plottedPoints,
  render,
  withUncertainty,
  yDomain,
  type PlottedPoint,
} from '../src/sections/cold-resistance.ts';

/**
 * A fixture from a `--mock` run, loaded as the page would.
 *
 * Found from the working directory, the package's, because under jsdom
 * `import.meta.url` isn't the test file's location.
 */
function fixture(name: string): Characterisation {
  const state = parse(readFileSync(join('test', 'fixtures', `${name}.json`), 'utf8'));
  if (state.kind !== 'loaded') {
    throw new Error(`${name} didn't load: ${JSON.stringify(state)}`);
  }
  return state.characterisation;
}

/** The number of points drawn in `container`'s graph. */
function dots(container: HTMLElement): number {
  return container.querySelectorAll('g.point circle').length;
}

/** The number of hollow markers showing corrected points as measured. */
function measuredMarkers(container: HTMLElement): number {
  return container.querySelectorAll('g.measured circle').length;
}

/** A plotted point, as measured, at `y` ± `yUncertainty` Ω and `x` A². */
function point(x: number, y: number, yUncertainty: number): PlottedPoint {
  return {
    setpoint: Math.sqrt(x),
    x,
    xUncertainty: 0,
    y,
    yUncertainty,
    measured: y,
    measuredUncertainty: yUncertainty,
    corrected: false,
  };
}

describe('the cold resistance graph', () => {
  it.each([
    ['before', 0],
    ['no-points', 0],
    ['partial', 4],
    ['complete', 9],
  ])('plots every analysed point in %s', (name, count) => {
    const container = document.createElement('div');
    render(container, fixture(name));
    expect(container.querySelector('svg')).not.toBeNull();
    expect(dots(container)).toBe(count);
  });

  it.each([
    ['before', 0],
    ['no-points', 0],
    ['partial', 0],
    ['complete', 9],
  ])('shows the points as measured in %s only once they are corrected', (name, count) => {
    const container = document.createElement('div');
    render(container, fixture(name));
    expect(measuredMarkers(container)).toBe(count);
  });

  it('draws empty axes before the file exists', () => {
    const container = document.createElement('div');
    render(container, null);
    expect(container.querySelector('svg')).not.toBeNull();
    expect(dots(container)).toBe(0);
  });

  it('replaces the previous graph when it renders again', () => {
    const container = document.createElement('div');
    render(container, fixture('partial'));
    render(container, fixture('complete'));
    expect(container.children).toHaveLength(1);
    expect(dots(container)).toBe(9);
  });

  it.each([
    ['before', false],
    ['no-points', false],
    ['partial', false],
    ['complete', true],
  ])('draws the fit in %s only once it has run: %s', (name, fitted) => {
    const container = document.createElement('div');
    render(container, fixture(name));
    expect(container.querySelector('.fit') !== null).toBe(fitted);
    expect(container.querySelector('.intercept') !== null).toBe(fitted);
  });

  // Plot reads a string channel as a field name, so a tooltip given as a
  // literal string looks up a field that doesn't exist and shows nothing.
  it("shows the fit's tooltip when the pointer is over the intercept", async () => {
    // Plot measures the tooltip's text to size its box, which jsdom can't do.
    SVGGraphicsElement.prototype.getBBox ??= () => ({ x: 0, y: 0, width: 100, height: 20 }) as DOMRect;

    const container = document.body.appendChild(document.createElement('div'));
    render(container, fixture('complete'));

    // The intercept's marker is drawn translated to its position.
    const marker = container.querySelector('g.intercept path[transform]')!;
    const [x, y] = marker
      .getAttribute('transform')!
      .match(/translate\(([^,]+),([^)]+)\)/)!
      .slice(1)
      .map(Number);
    container
      .querySelector('svg')!
      .dispatchEvent(new MouseEvent('pointermove', { clientX: x, clientY: y, bubbles: true }));

    // Plot draws the tooltip after the event, and each mark with tooltips has
    // its own group.
    await new Promise((resolve) => setTimeout(resolve, 50));
    const tip = [...container.querySelectorAll('g[aria-label="tip"]')]
      .map((group) => group.textContent)
      .join('');
    expect(tip).toContain('R₀ = ');
    expect(tip).toContain('b = ');
    container.remove();
  });

  it("doesn't plot a point whose analysis hasn't run", () => {
    const characterisation = fixture('partial');
    characterisation.cold_resistance!.points[0].analysis = null;
    expect(plottedPoints(characterisation)).toHaveLength(3);
  });
});

describe('plottedPoints', () => {
  it('plots the points as measured before the fit has run', () => {
    const characterisation = fixture('partial');
    const points = plottedPoints(characterisation);

    expect(points).toHaveLength(4);
    points.forEach((point, index) => {
      const measured = characterisation.cold_resistance!.points[index].analysis!.resistance_ohms;
      expect(point.corrected).toBe(false);
      expect([point.y, point.yUncertainty]).toEqual([measured.value, measured.uncertainty]);
      expect([point.measured, point.measuredUncertainty]).toEqual([
        measured.value,
        measured.uncertainty,
      ]);
    });
  });

  it('plots the corrected resistances once the fit has run, keeping the measured ones', () => {
    // The mock doesn't drift, so its corrections are 0: make one that isn't.
    const characterisation = fixture('complete');
    const coldResistance = characterisation.cold_resistance!;
    coldResistance.analysis!.corrected_resistances_ohms[2] = { value: 0.0955, uncertainty: 3e-5 };

    const points = plottedPoints(characterisation);
    const measured = coldResistance.points[2].analysis!.resistance_ohms;
    expect(points[2]).toMatchObject({
      y: 0.0955,
      yUncertainty: 3e-5,
      measured: measured.value,
      measuredUncertainty: measured.uncertainty,
      corrected: true,
    });
    points.forEach((point, index) => {
      const corrected = coldResistance.analysis!.corrected_resistances_ohms[index];
      expect(point.corrected).toBe(true);
      expect([point.y, point.yUncertainty]).toEqual([corrected.value, corrected.uncertainty]);
    });
  });
});

describe('plottedFit', () => {
  it("is null before the fit has run", () => {
    const characterisation = fixture('partial');
    expect(plottedFit(characterisation, plottedPoints(characterisation))).toBeNull();
  });

  it('is the fitted line, ending at the largest point', () => {
    const characterisation = fixture('complete');
    const points = plottedPoints(characterisation);
    const analysis = characterisation.cold_resistance!.analysis!;

    expect(plottedFit(characterisation, points)).toEqual({
      intercept: analysis.resistance_ohms.value,
      interceptUncertainty: analysis.resistance_ohms.uncertainty,
      slope: analysis.slope_ohms_per_amp_squared.value,
      slopeUncertainty: analysis.slope_ohms_per_amp_squared.uncertainty,
      end: Math.max(...points.map((point) => point.x)),
    });
  });
});

describe('yDomain', () => {
  it('is fixed at 0–2 Ω with no points', () => {
    expect(yDomain([], null)).toEqual([0, 2]);
  });

  it('runs from 1% below the lowest error bar to 1% above the highest', () => {
    const points = plottedPoints(fixture('complete'));
    const lowest = Math.min(...points.map((point) => point.y - point.yUncertainty));
    const highest = Math.max(...points.map((point) => point.y + point.yUncertainty));

    expect(yDomain(points, null)).toEqual([lowest * 0.99, highest * 1.01]);
  });

  it("includes the fit's intercept and its error bar", () => {
    // A self-heating filament: the line rises, so the intercept sits below
    // every point, and its combined uncertainty is wider than theirs.
    const points = [point(0.01, 0.1, 0.0001), point(0.09, 0.102, 0.0001)];
    const fit = {
      intercept: 0.09975,
      interceptUncertainty: 0.0025,
      slope: 0.025,
      slopeUncertainty: 0.001,
      end: 0.09,
    };

    expect(yDomain(points, fit)).toEqual([(0.09975 - 0.0025) * 0.99, (0.09975 + 0.0025) * 1.01]);
  });

  it('includes the measured resistances of corrected points', () => {
    // Corrected downwards by more than its error bar.
    const corrected = { ...point(0.04, 0.1, 0.0001), measured: 0.1005, corrected: true };
    expect(yDomain([corrected], null)).toEqual([(0.1 - 0.0001) * 0.99, 0.1005 * 1.01]);
  });
});

describe('withUncertainty', () => {
  it('gives the uncertainty to two significant figures, and the value to match', () => {
    expect(withUncertainty(0.0960004, 0.0000087)).toBe('0.0960004 ± 0.0000087');
    expect(withUncertainty(1.01234, 0.025)).toBe('1.012 ± 0.025');
    expect(withUncertainty(123.4, 5.6)).toBe('123.4 ± 5.6');
  });

  it("doesn't fail on a zero uncertainty", () => {
    expect(withUncertainty(0.1, 0)).toBe('0.1 ± 0');
  });
});
