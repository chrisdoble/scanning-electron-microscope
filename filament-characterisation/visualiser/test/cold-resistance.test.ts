// @vitest-environment jsdom

import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { parse } from '../src/data.ts';
import type { Characterisation } from '../src/generated/characterisation.ts';
import { plottedPoints, render, withUncertainty, yDomain } from '../src/sections/cold-resistance.ts';

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
  return container.querySelectorAll('circle').length;
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

  it("doesn't plot a point whose analysis hasn't run", () => {
    const characterisation = fixture('partial');
    characterisation.cold_resistance!.points[0].analysis = null;
    expect(plottedPoints(characterisation)).toHaveLength(3);
  });
});

describe('yDomain', () => {
  it('is fixed at 0–2 Ω with no points', () => {
    expect(yDomain([])).toEqual([0, 2]);
  });

  it('runs from 10% below the lowest error bar to 10% above the highest', () => {
    const points = plottedPoints(fixture('complete'));
    const lowest = Math.min(...points.map((point) => point.y - point.yUncertainty));
    const highest = Math.max(...points.map((point) => point.y + point.yUncertainty));

    expect(yDomain(points)).toEqual([lowest * 0.9, highest * 1.1]);
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
