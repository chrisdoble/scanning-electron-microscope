// Helpers shared by the page's sections.

/**
 * A caption saying what a section's graphs show, styled by the `caption` class.
 * Every section uses this rather than Plot's own caption, so they all look the
 * same, whether a section has one graph or several.
 */
export function caption(document: Document, text: string): HTMLParagraphElement {
  const paragraph = document.createElement('p');
  paragraph.className = 'caption';
  paragraph.textContent = text;
  return paragraph;
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
