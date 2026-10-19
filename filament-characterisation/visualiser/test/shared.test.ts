import { describe, expect, it } from 'vitest';
import { withUncertainty } from '../src/shared.ts';

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
