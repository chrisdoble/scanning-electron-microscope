# Hot sweep

This document specifies a new section of the characterisation procedure, measured after the cold resistance: the filament's resistance at currents up to a maximum and back down again. It builds on `1_ARCHITECTURE.md` and `2_COLD_RESISTANCE.md`, and everything there still applies unless this document says otherwise.

## Why

**R against I² at larger currents.** The cold resistance stops at 300 mA, where the filament barely warms. The sweep shows how its resistance changes as it heats, up to the currents it's operated at. That's what relates a heating current to the filament's state.

**Up, then down.** The sweep ramps up through a series of setpoints to a maximum, then back down through the same setpoints. The filament's own power warms its mount over the sweep. So at each setpoint, the resistance measured on the way down differs from the way up by how much the mount warmed in between. That difference is measured, not corrected.

**The maximum starts low and is raised over successive runs.** The filament is held in an epoxy mount, which softens at about 80 °C. The up/down difference at the low setpoints shows how much the mount warmed (see "Reading the results"), and the operator raises the maximum only while that stays acceptable. The maximum is a constant, edited in the code, and starts at 0.6 A.

**Not in scope:**
- **No polarity reversal.** The cold resistance reverses the current to cancel voltages that don't reverse with it, chiefly the scope's offset and thermal EMFs. At the sweep's voltages, hundreds of millivolts to volts, thermal EMFs are negligible, and the scope's offset is removed by reading it with no current (3.1). Reversal would also be costly here: the output is off for at least 50 ms while the relays switch, and a hot filament cools a lot in that time, so every point would need a second full settle.
- **No reference measurements or drift correction,** unlike the cold resistance (`4_TEMPERATURE_COMPENSATION.md`). The mount's warming is what the sweep measures.
- **No temperature.** Converting R to temperature isn't attempted. The filament's temperature isn't uniform along its length, and it's tungsten with 2% rhenium, not pure tungsten.
- **No power in the results.** P = VI is computed where it's plotted, from the analysis's voltage and current.

## Changes outside the procedure

### `constants.rs`

Raise `MAXIMUM_HEATING_CURRENT_AMPS` from 0.3 A to 3.0 A, channel 1's maximum. It then guards only against something the supply can't do, and the sweep's maximum is the limit that matters for the filament. Update its doc comment to say so.

### Splitting `characterisation.rs`

`procedure/characterisation.rs` is already about 1,700 lines, and the sweep would add several hundred more. Before adding it, split the file by section into a directory module, with no change in behaviour:

| File | Contents |
| --- | --- |
| `procedure/characterisation/mod.rs` | `characterise`, the short sections (`prepare`, `pump_down`, `spin_down`, `finish`), and the end-to-end mock tests, with their helpers (`run_against_mocks`, `answer_prompt`, …) |
| `procedure/characterisation/measurement.rs` | What both measurements use: `ramp_to`, `ramp_steps`, `change_polarity`, `check_conditions`, `sample`, `take_samples`, `wait_for_settle`, `is_settled`, `read_unclipped`, `unclipped_average`, `summarise`, `mean_and_standard_error` and `Settle`, the constants they use (e.g. `SAMPLE_COUNT`, `SETTLE_WINDOW`, `VERTICAL_SCALES_VOLTS_PER_DIVISION`, `CLIPPING_HEADROOM`, `OVERCURRENT_PROTECTION_MARGIN_AMPS`, `CURRENT_READBACK_RESOLUTION_AMPS`), and their unit tests |
| `procedure/characterisation/cold_resistance.rs` | Everything only the cold resistance uses: `measure_cold_resistance` and the functions, types and constants below it, and their unit tests |
| `procedure/characterisation/hot_sweep.rs` | The new section |

- **Visibility:** items shared between the files become `pub(super)`. Nothing outside `procedure/characterisation/` needs to change, since `procedure/mod.rs` only calls `characterise`.
- **Where a later measurement goes:** each new section gets its own file, and anything it shares with the others moves into `measurement.rs`.

### `procedure` module

Add `ProcedureError::Clipped(String)`, for a reading that clipped on the vertical scale in use.
- `sample` returns it in place of the `Check` error it returns for clipping now, with the same message.
- The cold resistance treats it exactly as before: it propagates, and ends the run.
- The sweep catches it and steps the scale up (3.2).

## Constants for the procedure

At the top of `hot_sweep.rs`, each with a doc comment.

| Constant | Value | Why |
| --- | --- | --- |
| `HOT_SWEEP_MAXIMUM_CURRENT_AMPS` | 0.6 | The sweep's maximum current. Raised by editing it, only once the previous maximum's up/down difference is acceptable (see "Reading the results"). |
| `HOT_SWEEP_VOLTAGE_LIMIT_VOLTS` | 5.0 | A hot filament at 3 A, with its leads and relays, needs a few volts. A low limit keeps the channel in constant current while limiting the energy delivered if the circuit opens and re-closes, as `COLD_RESISTANCE_VOLTAGE_LIMIT_VOLTS` does. |

The constants that choose the setpoints are only used by `hot_sweep_setpoints`, so they're declared inside it (see "Setpoints").

## Setpoints

`hot_sweep_setpoints(maximum: f64) -> (f64, Vec<f64>)` returns the step in I² and the currents going up. It's a pure function in `hot_sweep.rs`, so it can be unit tested.

**100 mA first.** The sweep always starts at 100 mA, the cold resistance's lowest current, where the filament's own heating is smallest. Its up/down difference is the best measure of how much the mount warmed (see "Reading the results"). Without it, the lowest setpoint would rise with the maximum: from 141 mA at 0.6 A to 447 mA at 1.6 A. It's an extra point below the grid, and isn't counted against the grid's limit. Nothing lower is worth measuring: below 100 mA the voltage is small and the filament barely warms.

**Evenly spaced in I².** The points are then evenly spaced on the R vs I² graph. At low current, where R is close to R₀, they're also roughly even in power.

**On a grid, so runs to different maxima share setpoints.** The setpoints are the multiples of a step in I², so a run to a higher maximum measures at the same currents as earlier runs, and the runs can be compared point by point.

**The step grows with the maximum,** so the sweep stays short.
- At about 15 s per point (a short ramp, a 4–7 s settle and 8 s of samples), a fixed 0.01 A² step would make a sweep to 1 A take about 50 minutes, and one to 1.6 A about 2 hours.
- A longer sweep also warms the mount more. So if the count grew with the maximum, part of the up/down difference's growth between maxima would come from the sweep getting longer, not from the higher currents.

**The rule:**
- The step is the smallest of 0.01, 0.02, 0.05, 0.1 and 0.2 A² that keeps the grid to at most 20 points, not counting the 100 mA point. These are two constants inside the function: `STEPS_AMPS_SQUARED` and `MAXIMUM_POINTS`.
- The setpoints are 100 mA, then √(k × step) for k = 1, 2, … up to the maximum, leaving out any at or below 100 mA. With a 0.01 A² step, k = 1 is 100 mA itself.
- If the maximum's I² isn't a multiple of the step, the maximum is added as the last point.
- When comparing I² with multiples of the step, allow for floating-point rounding: 0.6² / 0.02 is 17.999… in `f64`.

| Maximum | Step | Points up |
| --- | --- | --- |
| 0.6 A (0.36 A²) | 0.02 A² | 19: 100 mA, then 141 mA to 600 mA |
| 1.0 A | 0.05 A² | 21: 100 mA, then 224 mA to 1 A |
| 1.6 A (2.56 A²) | 0.2 A² | 14: 100 mA, then 447 mA to 1.6 A |

**The grids nest.** Every step divides 0.2 A², so multiples of 0.2 A² are setpoints at every maximum.

**The way down** is the setpoints going up, reversed, without the maximum, which isn't measured twice. With the 0.6 A maximum that's 19 points up and 18 down, about 9–10 minutes.

## The procedure

`characterise` gains a section, "Measuring the hot sweep", between "Measuring cold resistance" and "Finishing". It's an independent section: commenting out the cold resistance leaves a run that measures only the sweep.

### 3.0 Setup

1. Note the start time with `tokio::time::Instant`. Every point's time is relative to it.
2. Disable the output, set the current to 0 A, and set the voltage limit to `HOT_SWEEP_VOLTAGE_LIMIT_VOLTS`.
3. Set overcurrent protection to `HOT_SWEEP_MAXIMUM_CURRENT_AMPS` plus `OVERCURRENT_PROTECTION_MARGIN_AMPS`.
4. With the output still off, set the polarity to `Forward`. The whole sweep uses it.
5. Compute the setpoints, create the `HotSweep` (see "Results"), and save.

### 3.1 The scope's offsets

The scope's offset spec is ±(0.1 div + 2 mV). Its 0.1 div part grows with the vertical scale, and the scale grows with the voltage, so on any scale a reading of 2–3 divisions can be off by up to about 3–5%. Without reversal it has to be measured and subtracted.

- **With the output off,** read the scope on every scale in `VERTICAL_SCALES_VOLTS_PER_DIVISION`. That's 10 scales, so there's no need to guess which ones the sweep will use, and it works for the 1 Ω bench resistor too.
- **On each scale,** `set_vertical_scale`, then take `SAMPLE_COUNT` voltage readings with a new helper, `take_offset_samples`. It's like `take_samples`, but reads only the voltage, and doesn't check the regulation mode or overcurrent protection, since the output is off. It still runs `check_conditions` before each reading, and fails on clipping.
- **Summarise** each scale's readings with `summarise`, the existing helper that passes samples to `mean_and_standard_error.py` and returns a `Measurement` (the samples, their mean and its standard error). Show each as a measurement step (e.g. "Offset at 10 mV/div: −0.43 ± 0.01 mV"), record it, and save.
- **What the reading contains:** everything in the voltage path with no current, which is the scope's offset error, net thermal EMFs, and any DC pickup. All of it is subtracted.
- **It takes about 10 × 20 × 0.36 s ≈ 70 s,** while nothing is hot.

### 3.2 The sweep

Enable the output at 0 A. Then measure each setpoint going up, then each going down, each in its own sub-section titled e.g. "Measuring at 424 mA (up, 10/37)" or "Measuring at 424 mA (down, 28/37)". The points are numbered together, up and down, in the order measured, so the count shows how far through the whole sweep is, as the cold resistance numbers its setpoints and references together.

**At each setpoint:**
1. `ramp_to` the setpoint.
2. Set the vertical scale (see below).
3. `wait_for_settle`, then `take_samples(SAMPLE_COUNT)`, as in the cold resistance. `wait_for_settle`'s quick resistances are V/I in one polarity, which is what the sweep measures anyway.
4. Summarise the voltage and current with `summarise`.
5. Record the point, with its direction, setpoint, scale, settle time and time, and save. Then call `hot_sweep_point.py` (see "Analysis") with the point's voltage and current and its scale's offset. Record the analysis and save again, so the measurements survive an analysis that fails.
   - **The settle time** shows whether incomplete settling could be contributing to the up/down difference (see "Reading the results").
   - **The time,** the midpoint of `take_samples`' window as for the cold resistance, records the sweep's timing. The mount's warming at each point depends on how long the sweep took to get there, so a run that went unusually slowly, and warmed the mount more, shows up.
6. Show its resistance (e.g. "Resistance: 132.41 ± 0.02 mΩ") and its power, VI from the analysis (e.g. "Power: 41.2 mW").
7. **On the way down,** also show the change since the same setpoint on the way up, e.g. "Change since the way up: +0.31 mΩ (+0.23%)". This is the number the operator watches. It's only a difference, so it's fine in Rust.

**The vertical scale on the way up.**
- Each setpoint starts from the previous setpoint's scale. The voltage only rises going up, so the scale only ever needs to rise.
- After the ramp, wait `SETPOINT_APPLY_TIME`, then `read_unclipped`, which steps the scale up while the reading clips.
- The voltage rises a little as the filament settles at the new current. `CLIPPING_HEADROOM` leaves room for that.
- **If a reading clips anyway,** during the settle or the samples, `sample` returns `ProcedureError::Clipped`. Catch it, step the scale up, and measure the point again from step 3, discarding the partial point. Only the last scale's measurement is recorded. If there's no larger scale, fail with `Check`.

**The vertical scale on the way down.**
- At each setpoint, use the scale that setpoint was measured on going up.
- Each scale has its own gain error, up to ±2% under the pure-gain assumption (`2_COLD_RESISTANCE.md`, 3.4.3). Measuring a setpoint on the same scale both ways means the error cancels in the up/down difference.
- **If a reading clips going down,** step the scale up and measure the point again, as on the way up. Add a warning to the point that its up/down difference crosses scales, so it carries the two scales' gain errors.

**After the last setpoint,** ramp to 0 A, disable the output, and save.

## Analysis

`hot_sweep_point.py`, following the pattern of `cold_resistance_point.py` (`2_COLD_RESISTANCE.md`, 3.3). It reads a `HotSweepPointInput` on stdin and writes a `HotSweepPointAnalysis` on stdout, both defined in Rust with schemas generated from them (see "Results").

**Shared code.** It needs `cold_resistance_point.py`'s `AnalysisError`, `current_uncertainty` and `derived`, and `cold_resistance_fit.py` already duplicates `AnalysisError` and `derived`. Rather than one script importing from another, move all three into a new `python/analysis.py`, beside `script_io.py`, and have all three analysis scripts import from it. `test_cold_resistance.py`'s quantisation-floor test calls `analysis.current_uncertainty`.

**The input:**
- `voltage_volts` and `current_amps`, the point's `Measurement`s;
- `offset_voltage_volts`, the offset `Measurement` for the point's scale (3.1);
- `current_resolution_amps`.

**Using `uncertainties`:**
1. **The voltage, less the offset:** V = |V̄ − V̄₀|, each with its standard error. The absolute value is taken because the sign depends on how the relays and sense leads are wired, which the sweep doesn't need to know.
2. **The current:** I = Ī, with the quantisation floor, u(I) = √(SE² + Δ_I²/12), as in `cold_resistance_point.py`.
3. **The resistance:** R = V/I.
4. **Errors:** fail with a clear error if V is 0, or if u(R) = 0.

**The output:** `voltage_volts`, `current_amps` and `resistance_ohms`, each a `Derived`.

**What the uncertainty covers.** Each point's uncertainty is statistical: the samples' scatter, the current's quantisation, and the offset reading's own uncertainty.

**The systematic errors aren't propagated anywhere.** The scope's gain error is common to every point on a scale, and the supply's readback offset and gain errors are common to every point. There's no fit or budget for them to join, as there is for the cold resistance (`2_COLD_RESISTANCE.md`, 3.4).
- **The up/down difference at a setpoint isn't affected,** because both are measured on one scale with the same supply, so the errors cancel. That difference is the sweep's main output.
- **Absolute values carry them:** each R has a systematic uncertainty of about 1.2% from its scale's gain, and the readback offset adds about (5 mA / I)/√3, which is largest at low current: about 2.9% at 100 mA, and 0.5% at 600 mA.
- **The curve's shape can step** by up to a few percent where the scale changes between adjacent setpoints.
- **The visualiser's caption says this,** so the error bars aren't read as the whole uncertainty.

## Results

`Characterisation` gains `hot_sweep: Option<HotSweep>`, `None` until the sweep's setup has run. Fields are in alphabetical order with doc comments, as elsewhere in `results.rs`.

```rust
/// Everything recorded while measuring the hot sweep.
pub struct HotSweep {
    /// The sweep's maximum current in amperes.
    pub maximum_current_amps: f64,

    /// The scope's reading with no current on every vertical scale, in the
    /// order of `VERTICAL_SCALES_VOLTS_PER_DIVISION`, once read (3.1).
    pub offsets: Vec<HotSweepOffset>,

    /// One entry per setpoint, up then down, in the order measured.
    pub points: Vec<HotSweepPoint>,

    /// The setpoints' spacing in I², in A².
    pub step_amps_squared: f64,
}

/// The scope's reading with no current on one vertical scale.
pub struct HotSweepOffset {
    pub vertical_scale_volts_per_division: f64,
    pub voltage_volts: Measurement,
}

/// Which way the sweep was going when a point was measured.
pub enum HotSweepDirection {
    Down,
    Up,
}

/// The measurements at one setpoint, in one direction.
pub struct HotSweepPoint {
    /// The output of `hot_sweep_point.py`, once it's run.
    pub analysis: Option<HotSweepPointAnalysis>,

    pub current_amps: Measurement,
    pub direction: HotSweepDirection,

    /// How long the filament took to settle, in seconds.
    pub settle_seconds: f64,

    pub setpoint_amps: f64,

    /// The midpoint of the sampling window, in seconds since the sweep started.
    pub time_seconds: f64,

    pub vertical_scale_volts_per_division: f64,
    pub voltage_volts: Measurement,

    /// Warnings raised while measuring this point, e.g. a settle timeout, or a
    /// scale that differs from the way up.
    pub warnings: Vec<String>,
}

/// The analysis of one point: exactly the output of `hot_sweep_point.py`.
pub struct HotSweepPointAnalysis {
    pub current_amps: Derived,

    /// R = V/I.
    pub resistance_ohms: Derived,

    /// The voltage less the scale's offset, as a magnitude.
    pub voltage_volts: Derived,
}
```

- **Derives and placement:**
  - `HotSweepPointInput` and the output types derive `JsonSchema` and use `deny_unknown_fields`, as the cold resistance's do.
  - `HotSweepPointInput` lives in `python.rs`, beside the other script inputs.
  - `HotSweepDirection` derives `Clone`, `Copy`, `Debug`, `Deserialize`, `JsonSchema`, `PartialEq` and `Serialize`.
- **Regenerate:**
  - the results schema, with `results_schema_is_up_to_date`;
  - the Python schemas, adding `hot_sweep_point` to `schemas_are_up_to_date`'s list;
  - the visualiser's generated types.

## Visualiser

A new section, `sections/hot-sweep.ts`, after the cold resistance's in `main.ts`'s `SECTIONS`, following `sections/cold-resistance.ts`'s structure.

**Section headings.** With two sections, the page needs to say which graphs are which. `SECTIONS` becomes a list of `{ title, render }`, and `main.ts` draws each title as a heading, "Cold resistance" and "Hot sweep", with the section rendering into an element beneath it, so a render never touches its heading.

**Shared helpers.** A new `src/shared.ts` holds what the sections share:
- `withUncertainty`, moved from `sections/cold-resistance.ts`, with its tests moved to `test/shared.test.ts`;
- `caption(document, text)`, a paragraph with the `caption` class, styled in `style.css`. Every section's caption is one of these, beneath its graphs, rather than Plot's own caption, so the cold resistance's single graph and the hot sweep's two have captions that look the same.

The hot sweep draws two titled graphs:

- **"Resistance against current squared":**
  - x from 0 to a little past the maximum current squared, from `maximum_current_amps`, so points appear in place as they're measured;
  - y rescaled to the points, 1% beyond the outermost error bars, as the cold resistance's is.
- **"Resistance against power,"** with P = VI computed from each point's analysis, in milliwatts. The x axis runs from 0 to a little past the largest power, and the y axis is shared with the other graph so they can be compared.

**In both:**
- **Up and down are two series,** with distinct colours and marker shapes and a legend, so the up/down difference reads at a glance.
- **Error bars** are ±1 statistical standard uncertainty in R. One caption beneath both graphs says so, and that the scope's gain error and the supply's readback errors aren't included. Every section's caption is a paragraph made by `caption` in `shared.ts`, rather than Plot's own caption, so they all look the same.
- **A point's tooltip** shows its setpoint, direction, R ± u, P, and scale.
- **Points without an analysis** aren't plotted, as in the cold resistance section.

**Before the sweep has started**, `hot_sweep` is `None`. The section then draws empty axes, as the cold resistance's does before its points exist.

**Fixtures:** capture `complete.json` again from a `--mock` run, so it includes a sweep. Derive a new `sweep-partial.json` from it, cut back to a run quit part-way up the sweep. The mock filament's resistance doesn't change with current, so its sweep is flat. That's fine for testing the rendering.

## Reading the results

Not part of the implementation, but what the sweep is for.

**The up/down difference contains:**
- **The mount's warming,** which is what's wanted. At the lowest setpoints the filament's own heating is smallest, so the difference there approximates the mount's warming: ΔT ≈ ΔR / (R·α), with α ≈ 0.0045 /K.
- **Incomplete settling of the filament itself.** It reads low going up and high going down, the same sign as the mount's warming. `wait_for_settle` keeps it small, and the recorded settle times show whether it did.
- **A permanent change the first time the filament reaches a temperature.** A new filament anneals on its first heating, which changes its resistance permanently. The same can happen the first time it goes above an earlier maximum.
  - The next run's cold resistance shows whether it happened.
  - Repeat a run at a given maximum until consecutive cold resistances agree within their statistical uncertainties, and use that run's sweep.

**Raising the maximum:** if the low setpoints' up/down difference grows quickly from one maximum to the next, stop raising it.

**Resting between runs:** a sweep warms the mount far more than the cold resistance does. So the rest before the next run's cold resistance may need to be longer than 15 minutes. The cold resistance's reference drift display shows whether it was enough.

## Tests

### Rust

- **`hot_sweep_setpoints`:**
  - 0.6 A gives a step of 0.02 A² and 19 setpoints, 100 mA then 141 mA to exactly 0.6 A, despite 0.6² / 0.02 rounding to 17.999… in `f64`.
  - 1.0 A gives 0.05 A² and 21 setpoints.
  - 1.6 A gives 0.2 A² and 14 setpoints, the last 1.6 A.
  - The first setpoint is always 100 mA, never repeated, including with a 0.01 A² step, whose first multiple is 100 mA.
  - A maximum whose I² isn't a multiple of the step ends with the maximum as an extra point, and the grid never exceeds the point limit.
  - The setpoints increase, and every multiple of 0.2 A² up to the maximum is among them.
- **The end-to-end mock tests,** with and without vacuum:
  - the results have 10 offsets;
  - there are 19 points up and 18 down, every one analysed;
  - each down point's setpoint and scale match the up point at the same setpoint;
  - the times increase;
  - every resistance is 0.096 Ω, because the mock's offsets are 0 V and its resistance doesn't change with current.
- **Clipping:** the existing cold-resistance tests still pass with `sample` returning `ProcedureError::Clipped`.

The sweep adds about 120 Python calls to each end-to-end run, which will lengthen those tests by several seconds each.

### Python (`test_hot_sweep.py`)

1. **The offset is subtracted:** V̄ = 0.1 V with an offset of 0.002 V gives V = 0.098 V. Their standard errors add in quadrature.
2. **The sign doesn't matter:** a negative V̄ − V̄₀ gives the same R as a positive one of the same size.
3. **The quantisation floor:** a current with a standard error of 0 gets u = Δ_I/√12.
4. **Errors:** a voltage equal to its offset, or a zero uncertainty in R, exits non-zero with a clear message.
5. **Schemas:** the output validates against its schema, and invalid input is rejected, as for the other scripts.

### Visualiser

- **Every fixture renders,** and no data at all renders.
- **Counts:** the number of up and down points drawn matches each fixture.
- **Domains:** the R vs I² graph's x domain runs to the maximum current squared.
- **Power:** P is V × I from the analysis.

## Implementation order

Each commit builds, and passes `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, the Python tests and the visualiser's checks. Anything unused before the commit that uses it is marked `#[expect(dead_code)]`, naming that commit.

1. **Split `characterisation.rs`** into the directory module (see "Splitting `characterisation.rs`"). No behaviour change: every existing test passes unchanged.
2. **Setpoints.** `hot_sweep_setpoints` and its tests, and `MAXIMUM_HEATING_CURRENT_AMPS` raised to 3.0 A.
3. **Analysis and results.**
   - The results types and `HotSweepPointInput`.
   - `hot_sweep_point.py` and its tests, and the shared `analysis.py`.
   - Regenerated schemas and visualiser types.
4. **The procedure.**
   - `ProcedureError::Clipped`.
   - `take_offset_samples`.
   - The new section: setup, offsets, and the sweep with its display.
   - The end-to-end tests.
5. **The visualiser.** The section headings, `shared.ts`, `sections/hot-sweep.ts`, the recaptured and new fixtures, and their tests.
