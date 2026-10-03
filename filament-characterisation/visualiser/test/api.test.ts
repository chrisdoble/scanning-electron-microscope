import { createServer, type Server } from 'node:http';
import type { AddressInfo } from 'node:net';
import { mkdir, mkdtemp, rename, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { characterisationHandler } from '../api.ts';

describe('/api/characterisation', () => {
  let directory: string;
  let resultsPath: string;
  let server: Server;
  let url: string;

  beforeEach(async () => {
    directory = await mkdtemp(join(tmpdir(), 'visualiser-'));
    resultsPath = join(directory, 'characterisation.json');

    const handle = characterisationHandler(resultsPath);
    server = createServer((request, response) => void handle(request, response));
    await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
    url = `http://127.0.0.1:${(server.address() as AddressInfo).port}/api/characterisation`;
  });

  afterEach(async () => {
    await new Promise((resolve) => server.close(resolve));
    await rm(directory, { force: true, recursive: true });
  });

  // As the characteriser saves: a temporary file renamed over the old one.
  async function save(contents: string): Promise<void> {
    const temporary = `${resultsPath}.tmp`;
    await writeFile(temporary, contents);
    await rename(temporary, resultsPath);
  }

  it('is 404 before the file exists', async () => {
    const response = await fetch(url);
    expect(response.status).toBe(404);
  });

  it('returns the file with an ETag', async () => {
    await save('{"filament_id":null}');

    const response = await fetch(url);
    expect(response.status).toBe(200);
    expect(response.headers.get('content-type')).toBe('application/json');
    expect(response.headers.get('etag')).toMatch(/^".+"$/);
    expect(await response.text()).toBe('{"filament_id":null}');
  });

  it('is 304 while the file is unchanged', async () => {
    await save('{}');
    const etag = (await fetch(url)).headers.get('etag')!;

    const response = await fetch(url, { headers: { 'If-None-Match': etag } });
    expect(response.status).toBe(304);
    expect(await response.text()).toBe('');
  });

  it('returns the new contents once the file is saved again', async () => {
    await save('{}');
    const etag = (await fetch(url)).headers.get('etag')!;

    await save('{"filament_id":"F1"}');
    const response = await fetch(url, { headers: { 'If-None-Match': etag } });
    expect(response.status).toBe(200);
    expect(response.headers.get('etag')).not.toBe(etag);
    expect(await response.text()).toBe('{"filament_id":"F1"}');
  });

  it("is 500 with the error when the file can't be read", async () => {
    // A directory opens but can't be read as a file.
    await mkdir(resultsPath);

    const response = await fetch(url);
    expect(response.status).toBe(500);
    expect(await response.text()).toMatch(/EISDIR/);
  });
});
