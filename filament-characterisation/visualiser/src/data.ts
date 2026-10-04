// Fetches the results file from the server, validates it, and reports what the
// page should show.

import { Ajv2020 } from 'ajv/dist/2020.js';
import type { Characterisation } from './generated/characterisation.ts';
import schema from './generated/characterisation.schema.json';

/** What the page is showing, from the last response. */
export type PageState =
  /** The file doesn't exist yet. */
  | { kind: 'waiting' }
  | { kind: 'loaded'; characterisation: Characterisation }
  /** The file isn't valid JSON, or doesn't match the schema. */
  | { kind: 'invalid'; error: string }
  /** The server couldn't read the file. */
  | { kind: 'unreadable'; error: string }
  /** The server didn't answer. */
  | { kind: 'disconnected' };

// The formats schemars records, such as `double` and `uint64`, describe Rust's
// types rather than constrain the JSON, and Ajv doesn't know them.
const validate = new Ajv2020({ validateFormats: false }).compile<Characterisation>(schema);

/** The page's state for a results file with the given contents. */
export function parse(text: string): PageState {
  let data: unknown;
  try {
    data = JSON.parse(text);
  } catch (e) {
    return { kind: 'invalid', error: e instanceof Error ? e.message : String(e) };
  }

  if (!validate(data)) {
    const [error] = validate.errors ?? [];
    return {
      kind: 'invalid',
      error: error === undefined ? 'unknown error' : `${error.instancePath || '/'} ${error.message}`,
    };
  }

  return { kind: 'loaded', characterisation: data };
}

/**
 * Polls the server for the results file, calling `onState` whenever what the
 * page should show changes.
 *
 * Polls once a second, waiting a second after each response so slow responses
 * can't pile up. Asks with the last response's ETag, so an unchanged file costs
 * a `304` and calls nothing.
 */
export function poll(onState: (state: PageState) => void): void {
  const INTERVAL_MS = 1000;

  let etag: string | null = null;
  let last: string | null = null;

  // Reports a state other than `loaded`, but only if it differs from the last
  // one, so a missing file doesn't re-render the page every second.
  function report(state: Exclude<PageState, { kind: 'loaded' }>): void {
    const key = JSON.stringify(state);
    if (key !== last) {
      last = key;
      onState(state);
    }
  }

  async function tick(): Promise<void> {
    try {
      const response = await fetch('/api/characterisation', {
        headers: etag === null ? {} : { 'If-None-Match': etag },
      });

      if (response.status === 304) {
        // Unchanged.
      } else if (response.status === 404) {
        etag = null;
        report({ kind: 'waiting' });
      } else if (!response.ok) {
        etag = null;
        report({ kind: 'unreadable', error: await response.text() });
      } else {
        etag = response.headers.get('ETag');
        const state = parse(await response.text());
        if (state.kind === 'loaded') {
          last = null;
          onState(state);
        } else {
          report(state);
        }
      }
    } catch {
      // Forget the ETag, so that once the server answers again it sends the
      // file rather than a `304`, and the page leaves this state.
      etag = null;
      report({ kind: 'disconnected' });
    }

    setTimeout(() => void tick(), INTERVAL_MS);
  }

  void tick();
}
