A local web page that plots a results file written by the characteriser, updating as the file changes, so it can run beside the characteriser during a run.

See [`../docs/3_VISUALISER.md`](../docs/3_VISUALISER.md) for the design.

# Requirements

- [Node.js](https://nodejs.org/) 23.6 or later, which runs the server's TypeScript directly
- [pnpm](https://pnpm.io/)

# Running

1. Run `pnpm install` once.
2. Run `pnpm visualise <results file>`, e.g. `pnpm visualise ../characteriser/out/characterisation-1790830410.json`. A relative path is resolved from wherever you ran the command. The file doesn't have to exist yet.
3. Open http://127.0.0.1:5180.

# Developing

- `pnpm generate` regenerates `src/generated/` after the characteriser's results schema changes. CI checks it's up to date.
- `pnpm typecheck` checks the types.
- `pnpm test` runs the tests.

Edits to the page's sources reload in the browser while `pnpm visualise` is running. Edits to the server need a restart.
