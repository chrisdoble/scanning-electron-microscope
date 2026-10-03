// The visualiser's local server: the page, through Vite, and the results file.
//
// Run with `pnpm visualise <results file>`. Node runs this TypeScript directly,
// stripping the types, so there's no build step.

import { createServer } from 'node:http';
import { resolve } from 'node:path';
import { createServer as createViteServer, type Plugin } from 'vite';
import { characterisationHandler } from './api.ts';

const HOST = '127.0.0.1';
const PORT = 5180;

const [argument] = process.argv.slice(2);
if (argument === undefined) {
  console.error('usage: pnpm visualise <results file>');
  process.exit(1);
}

// pnpm runs scripts from the package's directory, and passes the directory the
// command was run from as `INIT_CWD`, so a relative path means what the user
// expects wherever they ran it.
const resultsPath = resolve(process.env.INIT_CWD ?? process.cwd(), argument);

const server = createServer();

const vite = await createViteServer({
  appType: 'spa',
  plugins: [resultsPathPlugin(resultsPath)],

  // Hot reload shares this server rather than opening a port of its own.
  server: { hmr: { server }, middlewareMode: true },
});

const handleCharacterisation = characterisationHandler(resultsPath);

server.on('request', (request, response) => {
  if (request.url?.split('?')[0] === '/api/characterisation') {
    void handleCharacterisation(request, response);
  } else {
    vite.middlewares(request, response);
  }
});

server.listen(PORT, HOST, () => {
  console.log(`Visualising ${resultsPath} at http://${HOST}:${PORT}`);
});

/** Puts the results file's path into the page, in place of `%RESULTS_PATH%`. */
function resultsPathPlugin(path: string): Plugin {
  const escaped = path
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;');

  return {
    name: 'results-path',
    transformIndexHtml: (html) => html.replace('%RESULTS_PATH%', escaped),
  };
}
