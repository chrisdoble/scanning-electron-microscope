import { readdirSync, readFileSync } from 'node:fs';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { parse, poll, type PageState } from '../src/data.ts';

// Results files from `--mock` runs, at each stage a run can stop.
const FIXTURES = new URL('./fixtures/', import.meta.url);

function fixture(name: string): string {
  return readFileSync(new URL(name, FIXTURES), 'utf8');
}

describe('parse', () => {
  // Also catches fixtures going stale when the results format changes.
  it.each(readdirSync(FIXTURES))('loads %s', (name) => {
    expect(parse(fixture(name)).kind).toBe('loaded');
  });

  it("reports a file that isn't JSON", () => {
    expect(parse('{').kind).toBe('invalid');
  });

  it("reports a file that doesn't match the schema", () => {
    const { in_vacuum, ...withoutField } = JSON.parse(fixture('complete.json'));
    const state = parse(JSON.stringify(withoutField));
    expect(state).toEqual({ kind: 'invalid', error: expect.stringContaining('in_vacuum') });
  });
});

describe('poll', () => {
  let responses: Array<Response | Error>;
  let requests: Array<Headers>;
  let states: PageState[];

  beforeEach(() => {
    vi.useFakeTimers();
    responses = [];
    requests = [];
    states = [];
    vi.stubGlobal('fetch', async (_url: string, init: RequestInit) => {
      requests.push(new Headers(init.headers));
      const response = responses.shift();
      if (response === undefined) {
        throw new Error('no response queued');
      }
      if (response instanceof Error) {
        throw response;
      }
      return response;
    });
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  function file(name: string, etag: string): Response {
    return new Response(fixture(name), { status: 200, headers: { ETag: etag } });
  }

  // Runs the first poll and then `count - 1` more, a second apart.
  async function run(count: number): Promise<void> {
    poll((state) => states.push(state));
    await vi.advanceTimersByTimeAsync(0);
    for (let i = 1; i < count; i++) {
      await vi.advanceTimersByTimeAsync(1000);
    }
  }

  it('waits for the file, then loads it', async () => {
    responses.push(new Response(null, { status: 404 }), file('partial.json', '"1"'));
    await run(2);
    expect(states.map((state) => state.kind)).toEqual(['waiting', 'loaded']);
  });

  it('reports waiting once, however many polls find no file', async () => {
    responses.push(...[1, 2, 3].map(() => new Response(null, { status: 404 })));
    await run(3);
    expect(states).toEqual([{ kind: 'waiting' }]);
  });

  it('asks with the ETag, and reports nothing while the file is unchanged', async () => {
    responses.push(file('partial.json', '"1"'), new Response(null, { status: 304 }));
    await run(2);
    expect(requests[1].get('If-None-Match')).toBe('"1"');
    expect(states.map((state) => state.kind)).toEqual(['loaded']);
  });

  it('loads the file again each time it changes', async () => {
    responses.push(file('partial.json', '"1"'), file('complete.json', '"2"'));
    await run(2);
    expect(states.map((state) => state.kind)).toEqual(['loaded', 'loaded']);
  });

  it("reports a file the server can't read", async () => {
    responses.push(new Response('EISDIR: illegal operation on a directory', { status: 500 }));
    await run(1);
    expect(states).toEqual([{ kind: 'unreadable', error: 'EISDIR: illegal operation on a directory' }]);
  });

  it('reports a lost connection, then reloads without the ETag once it returns', async () => {
    responses.push(file('partial.json', '"1"'), new TypeError('fetch failed'), file('partial.json', '"1"'));
    await run(3);
    expect(states.map((state) => state.kind)).toEqual(['loaded', 'disconnected', 'loaded']);
    expect(requests[2].has('If-None-Match')).toBe(false);
  });
});
