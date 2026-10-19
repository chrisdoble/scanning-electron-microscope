// @vitest-environment jsdom

import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { parse } from '../src/data.ts';
import type { Characterisation } from '../src/generated/characterisation.ts';
import {
  currentSquaredDomain,
  powerDomain,
  render,
  resistanceDomain,
  sweepPoints,
  type SweepPoint,
} from '../src/sections/hot-sweep.ts';

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

/** The number of `direction`'s points drawn in each of `container`'s graphs. */
function drawn(container: HTMLElement, direction: 'up' | 'down'): number[] {
  return [...container.querySelectorAll('figure')].map(
    (figure) => figure.querySelectorAll(`g.${direction} path, g.${direction} circle`).length,
  );
}

/** A plotted point at `resistance` ± `uncertainty` Ω and `power` mW. */
function point(resistance: number, uncertainty: number, power: number): SweepPoint {
  return {
    setpoint_amps: 0.3,
    direction: 'Up',
    current_squared_amps_squared: 0.09,
    power_milliwatts: power,
    resistance_ohms: resistance,
    resistance_uncertainty_ohms: uncertainty,
    vertical_scale_volts_per_division: 0.01,
  };
}

describe('the hot sweep graphs', () => {
  it.each([
    ['before', 0, 0],
    ['no-points', 0, 0],
    ['partial', 0, 0],
    ['sweep-partial', 7, 0],
    ['complete', 19, 18],
  ])('plot every analysed point in %s, up and down', (name, up, down) => {
    const container = document.createElement('div');
    render(container, fixture(name));
    expect(container.querySelectorAll('figure > svg')).toHaveLength(2);
    expect(drawn(container, 'up')).toEqual([up, up]);
    expect(drawn(container, 'down')).toEqual([down, down]);
  });

  it('draws empty axes before the file exists', () => {
    const container = document.createElement('div');
    render(container, null);
    expect(container.querySelectorAll('figure > svg')).toHaveLength(2);
    expect(drawn(container, 'up')).toEqual([0, 0]);
  });

  it('labels up and down in a legend', () => {
    const container = document.createElement('div');
    render(container, fixture('complete'));
    for (const figure of container.querySelectorAll('figure')) {
      const legend = [...figure.children]
        .filter((child) => child.tagName.toLowerCase() === 'div')
        .map((child) => child.textContent)
        .join('');
      expect(legend).toContain('Up');
      expect(legend).toContain('Down');
    }
  });

  it('shares one caption between the graphs', () => {
    const container = document.createElement('div');
    render(container, fixture('complete'));
    expect(container.querySelectorAll('figcaption')).toHaveLength(0);
    expect(container.querySelectorAll('p.caption')).toHaveLength(1);
  });

  it('titles each graph', () => {
    const container = document.createElement('div');
    render(container, fixture('complete'));
    const titles = [...container.querySelectorAll('figure h2')].map((title) => title.textContent);
    expect(titles).toEqual(['Resistance against current squared', 'Resistance against power']);
  });

  it('replaces the previous graphs when it renders again', () => {
    const container = document.createElement('div');
    render(container, fixture('sweep-partial'));
    render(container, fixture('complete'));
    expect(container.children).toHaveLength(3);
    expect(drawn(container, 'down')).toEqual([18, 18]);
  });
});

describe('sweepPoints', () => {
  it("doesn't plot a point whose analysis hasn't run", () => {
    // The last of its 8 points was saved but not yet analysed.
    const characterisation = fixture('sweep-partial');
    expect(characterisation.hot_sweep!.points).toHaveLength(8);
    expect(sweepPoints(characterisation)).toHaveLength(7);
  });

  it('computes the power as V × I from the analysis, in milliwatts', () => {
    const characterisation = fixture('complete');
    const points = sweepPoints(characterisation);
    characterisation.hot_sweep!.points.forEach((recorded, index) => {
      const analysis = recorded.analysis!;
      expect(points[index].power_milliwatts).toBe(
        analysis.voltage_volts.value * analysis.current_amps.value * 1000,
      );
      expect(points[index].current_squared_amps_squared).toBe(analysis.current_amps.value ** 2);
    });
  });
});

describe('currentSquaredDomain', () => {
  it('runs from 0 to just past the maximum current squared', () => {
    const characterisation = fixture('complete');
    const maximum = characterisation.hot_sweep!.maximum_current_amps;
    expect(currentSquaredDomain(characterisation)).toEqual([0, maximum * maximum * 1.05]);
  });

  it('is fixed before the sweep has started', () => {
    expect(currentSquaredDomain(fixture('partial'))).toEqual([0, 1]);
    expect(currentSquaredDomain(null)).toEqual([0, 1]);
  });
});

describe('powerDomain', () => {
  it('runs from 0 to just past the largest power', () => {
    expect(powerDomain([point(0.1, 0.0001, 10), point(0.12, 0.0001, 40)])).toEqual([0, 42]);
  });

  it('is fixed with no points', () => {
    expect(powerDomain([])).toEqual([0, 100]);
  });
});

describe('resistanceDomain', () => {
  it('runs from 1% below the lowest error bar to 1% above the highest', () => {
    const points = [point(0.1, 0.0001, 10), point(0.12, 0.0002, 40)];
    expect(resistanceDomain(points)).toEqual([(0.1 - 0.0001) * 0.99, (0.12 + 0.0002) * 1.01]);
  });

  it('is fixed with no points', () => {
    expect(resistanceDomain([])).toEqual([0, 2]);
  });
});
