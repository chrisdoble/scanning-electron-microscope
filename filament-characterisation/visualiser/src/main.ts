// The page: polls for the results file and renders each section from it.

import type { Characterisation } from './generated/characterisation.ts';
import { poll, type PageState } from './data.ts';
import * as coldResistance from './sections/cold-resistance.ts';

/**
 * Renders one part of the page from the results, or from `null` while there
 * are none, e.g. before the file exists.
 */
type Section = (container: HTMLElement, characterisation: Characterisation | null) => void;

/** The page's sections, in order. */
const SECTIONS: Section[] = [coldResistance.render];

const status = requireElement('#status');
const main = requireElement('#sections');

const containers = SECTIONS.map(() => main.appendChild(document.createElement('section')));

poll((state) => {
  const message = statusMessage(state);
  status.hidden = message === null;
  status.textContent = message;

  // Without a connection there's nothing new to show, so the last render
  // stays.
  if (state.kind === 'disconnected') {
    return;
  }

  const characterisation = state.kind === 'loaded' ? state.characterisation : null;
  SECTIONS.forEach((render, i) => render(containers[i], characterisation));
});

/** What the status line says in `state`, or `null` if it's hidden. */
function statusMessage(state: PageState): string | null {
  switch (state.kind) {
    case 'waiting':
    case 'loaded':
      return null;
    case 'invalid':
      return `This isn't a results file this version understands: ${state.error}`;
    case 'unreadable':
      return state.error;
    case 'disconnected':
      return 'Lost the connection to the visualiser server';
  }
}

function requireElement(selector: string): HTMLElement {
  const element = document.querySelector(selector);
  if (!(element instanceof HTMLElement)) {
    throw new Error(`required element not found: ${selector}`);
  }
  return element;
}
