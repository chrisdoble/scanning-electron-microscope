# Results visualiser

A small web app that plots the cold-resistance data in a `Characterisation` results file written by the characteriser. It re-reads the file every second, so it can run beside the characteriser as a second view of a run in progress, or open a finished run afterwards.

It lives at `filament-characterisation/visualiser/`, a sibling of the characteriser, as the electron gun simulation keeps its web app beside its crates.

## Goals

- **Plot the cold resistance.** That's R against $I^2$ for whichever setpoints have been measured, with error bars, plus the fitted line once the fit has run.
- **Handle partial files.** A file is saved at every checkpoint, so it may stop anywhere: before the cold resistance starts, part-way through the setpoints, or before the fit. Plot whatever is there.
- **Update live.** Changes to the file appear within about a second while a run is in progress.
- **Extend to future measurements.** The heating-behaviour data planned for higher currents should slot in as a new section without restructuring anything.

## Non-goals

- **No tables or summaries.** The page is the graph and the path of the file it's showing. The fit's numbers, the uncertainty budget and the per-setpoint details are in the characteriser's display and the results file.
- **No calculation.** The visualiser plots what the characteriser and its Python scripts recorded. It doesn't fit, average or propagate uncertainties itself, following ARCHITECTURE.md's rule that statistics happen in Python. The one exception is drawing the fitted line, $R_0 + b\,x$, from the stored $R_0$ and $b$.
- **No editing, and no control of the characteriser.** It only reads.
- **Only the current results format.** Files from older versions of the characteriser, such as the stub's four `cold_*` fields, fail validation and are reported as such, not plotted.
- **Local use only.** The server listens on `127.0.0.1:5180`.

## Running it

From `filament-characterisation/visualiser/`:

```
pnpm install
pnpm visualise ../characteriser/out/characterisation-1790830410.json
```

Then open `http://127.0.0.1:5180`. The path is resolved against the directory the command was run from, which pnpm passes to the script as `INIT_CWD`. That way, the same command works from the repo root with `pnpm --dir filament-characterisation/visualiser visualise <path>`.

The file doesn't have to exist yet. You can start the visualiser with the path the characteriser printed and logged ("Writing results to …") before the first save. The page shows an empty graph until the file appears.

## Architecture

```
filament-characterisation/visualiser/
  package.json          // scripts: visualise, generate, test, typecheck
  tsconfig.json
  index.html
  server.ts             // the local server: Vite in middleware mode, plus the data endpoint
  api.ts                // the data endpoint, separate so its tests needn't start the server
  src/
    main.ts             // polling loop, validation, dispatch to the sections
    data.ts             // fetching with ETags, and the page's load states
    sections/
      cold-resistance.ts// the R vs I² graph
    generated/
      characterisation.ts  // TypeScript types generated from the schema (committed)
    style.css
  test/                 // vitest: fixtures and rendering tests
  README.md
```

### The server (`server.ts`)

A single Node process. Node 23.6+ runs TypeScript directly by stripping types, so there's no build step.

- **The page:** Vite runs in middleware mode (`createServer({ server: { middlewareMode: true } })`). It serves `index.html` and the TypeScript sources with hot reload, so editing the visualiser while it's running just works. Nothing is built or deployed.
- **The data:** `GET /api/characterisation` reads the file given on the command line and returns it.
  - **ETag:** the response carries one made from the file's modification time and size. A request with a matching `If-None-Match` gets `304 Not Modified` with no body, so unchanged polls are cheap.
  - **Missing file:** `404`.
  - **Unreadable file:** `500` with the error message.
- **Atomic saves:** the characteriser saves by writing a temporary file and renaming it over the old one. A read therefore always sees a complete file, never one half-written. The server needs no file watching, so it doesn't matter that a rename replaces the file the path points at.

### The page (`main.ts`, `data.ts`)

- **Polling:** the page fetches `/api/characterisation` every second with `If-None-Match`, then waits a second after each response, so slow responses never pile up. That pace matches the characteriser's: nothing happens faster than its once-a-second hardware poll.
- **Validation:** a changed file is parsed and validated against `characterisation.schema.json` with Ajv (`ajv/dist/2020`, since schemars generates draft 2020-12 schemas). Once validated, it's typed as the generated `Characterisation`, so every field access is checked by the compiler.
- **Rendering:** each section is re-rendered from the new data. Nothing is patched incrementally: there's little enough data that rebuilding is instant, and it keeps every section a pure function of the file.
- **The header:** the page shows the results file's path. Below it, a status line appears only when something is wrong:

| State | Shown |
| --- | --- |
| Waiting | No status line; the sections render as if every field were empty |
| Loaded | No status line |
| Invalid | "<path> isn't a results file this version understands", with Ajv's first error |
| Unreadable | The server's error |
| Disconnected | "Lost the connection to the visualiser server", keeping the last good render on screen |

### Sections

Each section is a module exporting one function:

```ts
export function render(container: HTMLElement, characterisation: Characterisation | null): void;
```

It owns its part of the page and draws something sensible from whatever data there is, including none: `null` while the file doesn't exist yet. `main.ts` keeps an ordered list of sections and calls each in turn. Adding the heating-behaviour work means adding an `Option` field to `Characterisation`, regenerating the types, and adding a section module to the list.

### Plotting: Observable Plot

The charts use [Observable Plot](https://observablehq.com/plot/) (`@observablehq/plot`), which is a few hundred kB including the parts of D3 it uses.

- **Why it fits:** it's declarative (a chart is a list of marks over data), renders SVG, and handles scales, axes and tooltips. Error bars are two rule marks with tick ends. It returns a plain DOM element, which suits vanilla TypeScript and full re-rendering.
- **Considered and rejected:**
  - **Plotly.js** has built-in error bars and zoom, but it's around 3.5 MB, and its imperative API is more than these plots need.
  - **Chart.js** needs a plugin for error bars.
  - **Raw D3** means writing axes and scales by hand.

If zooming turns out to matter for the heating data, Plotly is the fallback. A section's choice of library is private to it.

## Sharing the data format

The characteriser's Rust types are the source of truth, as they are for the Python scripts.

- **Schema derives:** `Characterisation`, `ColdResistance` and `ColdResistancePoint` gain `JsonSchema` derives. The types they contain already have them.
- **The committed schema:** a new snapshot test in `results.rs`, `results_schema_is_up_to_date`, generates `schema_for!(Characterisation)`. It compares that with the committed `filament-characterisation/characteriser/schemas/characterisation.schema.json`, and rewrites it when run with `UPDATE_SCHEMAS=1`, like the Python schemas test.
- **Generated types:** `pnpm generate` runs `json-schema-to-typescript` on that schema to produce `src/generated/characterisation.ts`, which is committed.
- **Drift:** CI regenerates the types and fails if the result differs from what's committed.

`Option` fields become `T | null`, which is exactly what makes partial files type-check: every field the characteriser may not have reached yet has to be handled.

## The graph (`sections/cold-resistance.ts`)

R against $I^2$:

- **Units:** R in ohms on the $y$ axis, $I^2$ in A² on the $x$ axis. Both axes start at 0, so the intercept is in view.
- **The $x$ axis is fixed at 0–0.095 A².** That's just over 0.09 A², the largest setpoint squared (300 mA). It never rescales, so points appear in place as they're measured.
- **The $y$ axis:**
  - **With no points,** it's fixed at 0–2 Ω. That covers the 0.1 Ω filaments and the 1 Ω bench-test resistor.
  - **Once there are points,** it rescales to them, from 0 to just above the largest point plus its error bar. On a fixed 0–2 Ω axis, a 0.1 Ω filament's points would sit in the bottom 5% of the graph, and its error bars would be far too small to see.
- **When `cold_resistance` is `null`, or the file doesn't exist yet:** the empty axes, with no message.
- **Points:** every point whose `analysis` is present, at $x$ = `current_squared_amps_squared` and $y$ = `resistance_ohms`.
  - A point without an `analysis` isn't plotted, since its resistance was never calculated. That only happens briefly, between a point being saved and its analysis finishing.
- **Error bars:** each point has both, at ±1 standard uncertainty (k = 1), and the axis labels say so. The $x$ bars are usually too small to see, but they're drawn, because they're recorded. Points with warnings are drawn like any other.
- **Tooltips:** each point shows its setpoint, $R \pm u$ and $I^2 \pm u$.
- **The fit**, once `analysis` is present:
  - the line $R_0 + b\,x$ (`resistance_ohms` and `slope_ohms_per_amp_squared` values), drawn from $x = 0$ to the largest point;
  - its intercept $R_0$, marked at $x = 0$, with an error bar of ±$u_c(R_0)$. That's the combined uncertainty, which is wider than the points' statistical ones.
  - The line's tooltip shows $R_0 \pm u_c$ and $b \pm u(b)$.
- **Before the fit:** the points alone.

## Testing

- **Fixtures:** `test/fixtures/` holds results files captured from `--mock` runs, at four stages:
  - before the cold resistance;
  - after the scale search with no points;
  - part-way through the setpoints;
  - complete.

  A test validates every fixture against the committed schema, so they fail loudly when the format changes rather than going stale.
- **Rendering tests** (vitest with jsdom) render each fixture, and no data at all, and check the graph: the number of points plotted, whether the fit line and intercept are drawn, and the axes' domains (fixed when empty, rescaled to the points otherwise).
- **The server:** a test of the data endpoint covers the 404, 200 and 304 responses and the ETag.
- **CI:** a `visualiser` job in `.github/workflows/ci.yml` runs `pnpm install`, `pnpm typecheck` (`tsc --noEmit`), `pnpm test`, and the generated-types check.

## Implementation order

The work splits into five commits. Each leaves something that builds, passes its checks and runs, and each can be reviewed on its own. The first is Rust only; the rest are confined to `filament-characterisation/visualiser/`.

1. **Publish the results schema from the characteriser.**
   - Add the `JsonSchema` derives on `Characterisation`, `ColdResistance` and `ColdResistancePoint`.
   - Add the `results_schema_is_up_to_date` snapshot test, and commit the generated `schemas/characterisation.schema.json`.
   - Done when the characteriser's tests, clippy and fmt pass. Nothing changes at run time.
2. **Serve the file.**
   - Create the package: `package.json`, `tsconfig.json`, and `index.html` showing the path.
   - Add `server.ts` (Vite in middleware mode) and `api.ts` (`/api/characterisation` with its ETag, 304, 404 and 500 handling).
   - Add the server's tests.
   - Add the `visualiser` CI job, running `pnpm install`, `pnpm typecheck` and `pnpm test`.
   - Check: `pnpm visualise <path>` against a `--mock` run serves the page, and `/api/characterisation` returns the file as it changes.
3. **Load and validate the data.**
   - Add `pnpm generate` and the committed generated types, and the generated-types check in CI.
   - Add the polling loop in `data.ts`, Ajv validation, and the status line for each page state.
   - Capture the fixtures from `--mock` runs. Quitting part-way through a run saves the partial file. Add the test that validates the fixtures against the schema.
   - Add the empty section list in `main.ts`.
   - Check: the page tracks a running `--mock` run, and shows the right status for a missing, invalid or unreadable file, and after stopping the server.
4. **Plot the points.**
   - Add `sections/cold-resistance.ts` with Observable Plot:
     - the fixed $x$ axis;
     - the $y$ axis fixed when empty and rescaled to the points otherwise;
     - the points with both error bars;
     - the tooltips.
   - Add rendering tests for every fixture and for no data.
   - Check: a `--mock` run's points appear one by one as each setpoint is analysed.
5. **Draw the fit.**
   - Add the fitted line, the intercept with its combined-uncertainty error bar, and the line's tooltip.
   - Extend the rendering tests to check the fit appears only once there's an `analysis`.
   - Check: the line appears when a `--mock` run finishes.

Steps 2 and 3 could be merged if a page that only serves raw JSON doesn't seem worth a commit. Keeping them apart separates the server, which is plain Node, from the page's data handling.

## Future: heating behaviour

When the heating characterisation is designed, it will add its own `Option` field to `Characterisation`, for example a time series of current, voltage and resistance per setpoint. The visualiser then gains a `sections/heating.ts`. The polling, validation, type generation, page states and handling of missing data all carry over unchanged.
