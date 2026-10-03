// The data endpoint: serves the results file the visualiser was started with.

import type { IncomingMessage, ServerResponse } from 'node:http';
import { open } from 'node:fs/promises';

/**
 * Handles `GET /api/characterisation`, returning the file at `resultsPath`.
 *
 * - `200` with the file's contents and an ETag made from its modification time
 *   and size.
 * - `304` with no body if the request's `If-None-Match` matches, so the page's
 *   once-a-second polls cost almost nothing while the file is unchanged.
 * - `404` if the file doesn't exist yet, e.g. before the characteriser's first
 *   save.
 * - `500` with the error message if it can't be read.
 *
 * The characteriser saves by writing a temporary file and renaming it over the
 * old one, so a read always sees a complete file. The file is opened once and
 * both its metadata and contents come from that handle, so the ETag always
 * describes the contents sent with it, even if a save lands mid-request.
 */
export function characterisationHandler(
  resultsPath: string,
): (request: IncomingMessage, response: ServerResponse) => Promise<void> {
  return async (request, response) => {
    if (request.method !== 'GET' && request.method !== 'HEAD') {
      response.writeHead(405, { Allow: 'GET, HEAD' }).end();
      return;
    }

    let file;
    try {
      file = await open(resultsPath);
    } catch (e) {
      if (isNotFound(e)) {
        response.writeHead(404, { 'Content-Type': 'text/plain' }).end(`${resultsPath} doesn't exist yet`);
      } else {
        respondWithError(response, e);
      }
      return;
    }

    try {
      const stat = await file.stat();
      const etag = `"${stat.mtimeMs}-${stat.size}"`;
      const headers = { 'Cache-Control': 'no-cache', ETag: etag };

      if (request.headers['if-none-match'] === etag) {
        response.writeHead(304, headers).end();
        return;
      }

      const contents = await file.readFile();
      response.writeHead(200, { ...headers, 'Content-Type': 'application/json' });
      response.end(request.method === 'HEAD' ? undefined : contents);
    } catch (e) {
      respondWithError(response, e);
    } finally {
      await file.close();
    }
  };
}

function isNotFound(e: unknown): boolean {
  return e instanceof Error && 'code' in e && e.code === 'ENOENT';
}

function respondWithError(response: ServerResponse, e: unknown): void {
  const message = e instanceof Error ? e.message : String(e);
  response.writeHead(500, { 'Content-Type': 'text/plain' }).end(message);
}
