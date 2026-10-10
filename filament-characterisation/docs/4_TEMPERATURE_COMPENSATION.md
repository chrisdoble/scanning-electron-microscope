# Compensating for the filament mount's temperature

This document specifies how the cold-resistance measurement (`2_COLD_RESISTANCE.md`) compensates for the filament's mount warming during a run. It adds reference measurements at a fixed current through the run, and the fit uses them to remove the mount's drift from every other measurement before fitting $R$ against $I^2$.

Everything in `1_ARCHITECTURE.md` and `2_COLD_RESISTANCE.md` still applies unless this document says otherwise.

## Why

### The problem

Three cold-resistance runs on the same filament gave $\chi^2_\nu$ of 75, 63 and 65, where about 1 is expected. The residuals look random when sorted by current. In measurement order, though, they follow a common pattern: the first point is 8–17σ low, and the rest rise through the run.

The third run measured its first setpoint again at the end, and the resistance had risen by 362 µΩ (+0.37%).

### The cause

The filament's own power warms its legs and mount, which in vacuum can only lose heat slowly by conduction to the flange. As the mount warms, the whole filament's temperature profile shifts up, and every reading rises with it. The setpoints are measured in random order, so this rise over time appears as scatter about the fitted line rather than as a bias in its slope.

A 30-minute hold test (`host/src/bin/hold.rs`) confirmed it:

| Hold | Change in R | Time constant τ |
| --- | --- | --- |
| 300 mA for 15 min | +942 µΩ (+0.94%) | 153 s |
| Then 100 mA for 15 min | −851 µΩ (−0.83%) | 160 s |

These come from fitting a single exponential to each hold.

- **It's thermal.** The change reverses with the power, which instrument drift wouldn't do.
- **It's not the chamber.** The second and third runs drifted about equally, though the chamber warmed 0.3 K in one and 2.0 K in the other.
- **There's probably a second, slower component,** with a time constant of tens of minutes. R was still moving at the end of both holds. Adding a linear term to the fit gives 9–17 µΩ/min of slow drift, and moves the fast time constant to anywhere from 95 to 190 s, so τ is only known roughly.

### What this changes

- **R₀:** the drift moves it by about 0.1–0.4 mΩ (0.1–0.4%). That's small next to its ±2.5 mΩ uncertainty, which the supply's readback offset dominates.
- **The slope b:** it varies about ±6% between runs because of the drift.
- **χ²_ν:** it's large in every run, so the poor-fit warning fires every time and stops meaning anything.
- **Later work:** once the readback offset is calibrated, and for the heating characterisation, the drift becomes one of the largest errors.

### The approach

Resting between setpoints was rejected: with τ ≈ 155 s, letting the mount cool fully would add 1.5–2 hours to a run. Warming up first was rejected because it biases R₀ high by the equilibrium warming. A physical thermal model was rejected for now, because it depends on a model being right (see "Alternatives considered").

Instead, **reference measurements at a fixed current**:
- one at the start, while the mount is still cold;
- more through the run, and one at the end.

Comparing the reference readings with the first gives the mount's drift over time. Interpolating between them gives the drift at the moment each setpoint was measured, which is then subtracted from that setpoint before fitting. This makes no assumption about the drift's shape. R₀ then describes the filament with its mount at the start temperature.

## The correction

### Why the same amount is subtracted at every current

Near room temperature, tungsten's resistivity is $\rho(T) = \rho_0[1 + \alpha(T - T_0)]$. A rise of $\Delta T$ adds a fixed amount, $\rho_0\alpha\,\Delta T$, however warm the filament already is.

A warming mount raises the filament's whole temperature profile, legs and centre alike, by its own $\Delta T$. That follows from the linearity of heat transport, the same argument as in `2_COLD_RESISTANCE.md`'s opening section. So for a filament of cross-section $A$:

$$R' = \frac{1}{A}\int \left[\rho(T(x)) + \rho_0\alpha\,\Delta T\right] dx = R + R_0\,\alpha\,\Delta T$$

The resistance rises by the same number of ohms at every current. A reference current repeated over time therefore measures exactly the amount to subtract from every other setpoint measured at the same moment:

$$\Delta R(t) = R_{ref}(t) - R_{ref}(t_0)$$

The self-heating at the reference current is the same every time it's measured: same current, same settling. So it cancels in that difference.

**What's ignored:**
- α's variation over a few kelvin;
- thermal expansion, which changes resistance about 1,000 times less than α does;
- the second-order effect of a slightly warmer filament dissipating slightly more power.

### Interpolating the drift

References are measured at times $t_0 < t_1 < \dots < t_m$. A setpoint measured at time $t_i$, between references $j$ and $j+1$, gets the drift interpolated linearly between them:

$$w_i = \frac{t_i - t_j}{t_{j+1} - t_j}, \qquad \Delta R(t_i) = (1 - w_i)\,R_{ref,j} + w_i\,R_{ref,j+1} - R_{ref,0}$$

and its corrected resistance is

$$R'_i = R_i - \Delta R(t_i) = R_i - (1 - w_i)\,R_{ref,j} - w_i\,R_{ref,j+1} + R_{ref,0}$$

- **Linear interpolation** is used because it assumes nothing about the drift's shape and can't overshoot between references.
- **Every setpoint has a reference before and after it,** because references are measured first and last (see "The schedule"). A point outside the references' time range is an error, not something to extrapolate.
- **The time of a setpoint, $t_i$,** is the mean of the midpoints of its two polarities' sampling windows. If the positive polarity's samples are taken from $t^+_{start}$ to $t^+_{end}$, and the negative polarity's from $t^-_{start}$ to $t^-_{end}$:

  $$t_i = \frac12\left(\frac{t^+_{start} + t^+_{end}}{2} + \frac{t^-_{start} + t^-_{end}}{2}\right) = \frac{t^+_{start} + t^+_{end} + t^-_{start} + t^-_{end}}{4}$$

  A point's $R$ weights the two polarities equally, so for a drift that's linear over the point, this time is exact. A reference's time is calculated the same way.

### Interpolation error

The interpolated $\Delta R(t_i)$ is only an estimate of the drift at $t_i$. It's a straight line between the references, but the true drift, $f(t)$, curves: it rises quickly at first, then levels off. So each subtraction is slightly wrong, by more where the drift curves sharply or the references are far apart.

That error is part of each corrected point's uncertainty. It's estimated as $u_{interp,i}$ below, and added in quadrature to $u^2(R'_i)$, the diagonal of the covariance matrix the fit uses (see "Propagating the uncertainty"). Without it, the fit would treat the correction as exact, and the leftover curvature would show up as excess χ².

For a linear interpolant, the error's size is $\tfrac12 |f''(\xi)|(t - t_j)(t_{j+1} - t)$ for some $\xi$ in the interval. It's zero at the references and largest midway between them.

- **Estimating the curvature:** $f''$ is estimated from each interval's neighbouring references, using the second divided difference of three consecutive references:

  $$f'' \approx 2\left[\frac{R_{ref,k+1} - R_{ref,k}}{t_{k+1} - t_k} - \frac{R_{ref,k} - R_{ref,k-1}}{t_k - t_{k-1}}\right] \Big/ (t_{k+1} - t_{k-1})$$

  Each interval uses the larger magnitude of the estimates centred on its two ends.
- **The first interval:** the first reference has no estimate centred on it, because no reference comes before it. That's where the drift curves most, so the next estimate alone would understate it. Instead, its estimate is extrapolated back from the two estimates centred on references 1 and 2, $c_1$ and $c_2$, assuming the curvature shrinks geometrically, as an exponential approach's does:

  $$c_0 = \max\left(c_1,\; c_1\left(\frac{c_1}{c_2}\right)^{(t_1 - t_0)/(t_2 - t_1)}\right)$$

  **Where the extrapolation comes from.** Here $c_k$ is the estimate centred on reference $k$, roughly the average of $|f''|$ from $t_{k-1}$ to $t_{k+1}$. For an exponential approach, $f(t) = a - b\,e^{-t/\tau}$, so

  $$f''(t) = -\frac{b}{\tau^2}e^{-t/\tau}, \qquad |f''(t)| = \frac{b}{\tau^2}e^{-t/\tau}$$

  The curvature at a time $\Delta$ later, relative to that at $t$, is

  $$\frac{|f''(t + \Delta)|}{|f''(t)|} = e^{-\Delta/\tau}$$

  That doesn't depend on $t$: the curvature shrinks by the same factor over any stretch of length $\Delta$, wherever it is in the run. Taking each ratio as later over earlier:

  - $c_2$ is $t_2 - t_1$ after $c_1$, so $c_2/c_1 = e^{-(t_2 - t_1)/\tau}$, and so $c_1/c_2 = e^{(t_2 - t_1)/\tau}$.
  - $c_1$ is $t_1 - t_0$ after $c_0$, so $c_1/c_0 = e^{-(t_1 - t_0)/\tau}$, and so $c_0 = c_1\,e^{(t_1 - t_0)/\tau}$.

  τ isn't known. But writing the second exponent as a fraction of the first,

  $$e^{(t_1 - t_0)/\tau} = \left(e^{(t_2 - t_1)/\tau}\right)^{(t_1 - t_0)/(t_2 - t_1)} = \left(\frac{c_1}{c_2}\right)^{(t_1 - t_0)/(t_2 - t_1)}$$

  gives $c_0 = c_1\,(c_1/c_2)^{(t_1 - t_0)/(t_2 - t_1)}$, without τ. The drift levels off, so $c_1 > c_2$, and the extrapolated $c_0$ is larger than $c_1$, as the curvature at the start should be. With the schedule's spacing, about 80 s then 120 s, the measured ratio is raised to the power ⅔.

  Treating each $c_k$, which is an average over a span, as the curvature at $t_k$ is an approximation, and the hold test's slower second component isn't a single exponential. The test against exponential drifts across a range of time constants (test 3) is what shows it works.

  With 400 µΩ of drift, this bounds the first interval's actual error for time constants from 60 to 250 s, e.g. 11.8 µΩ against an actual 10.3 µΩ at 155 s. The estimate centred on reference 1 alone gives 7.5 µΩ. It needs four references and a nonzero $c_2$, and otherwise falls back to $c_1$.
- **The last interval** has no estimate centred on the last reference either. But the drift has levelled off by then, so the one centred on the reference before is enough.
- **As an uncertainty:** the resulting bound $e_i = \tfrac12|f''|(t_i - t_j)(t_{j+1} - t_i)$ is treated as rectangular, so $u_{interp,i} = e_i/\sqrt3$. It's independent between points.
- **It's conservative.** Noise in the references inflates the second differences, and each interval takes the larger of two estimates. Noise can only make the first interval's extrapolation larger, because it's never less than $c_1$.
- **The size:** with τ ≈ 155 s, a run's drift amplitude of about 0.5 mΩ, and references about 120 s apart, the largest interpolation error is about 40 µΩ at the start of the run, where the drift curves most, falling as it levels off. That's comparable to a point's own uncertainty, which is why the schedule puts an extra reference early.

### Propagating the uncertainty

The corrected resistances are no longer independent:
- every one includes $+R_{ref,0}$;
- neighbouring points share their bracketing references.

So the fit uses their full covariance matrix, not just their individual uncertainties.

- **In the fit script,** every input is a `ufloat` from the `uncertainties` package: each point's $V$ and $I$, and each reference's $V$ and $I$, as in 3.3. The correction is computed with them, so the package tracks every correlation.
- **The covariance matrix** of the corrected $R'_i$ is `uncertainties.covariance_matrix(...)`, with $u_{interp,i}^2$ added to its diagonal.
- **The fit** is the same weighted least squares as 3.4.1, with that matrix as `sigma`. `curve_fit` takes a 2-D `sigma` as the data's covariance and does generalised least squares, with `absolute_sigma=True` as before. Its `info["fvec"]` is then the whitened residuals, so χ² is still `np.sum(info["fvec"] ** 2)` and 3.4.2 is unchanged.

**Example.** A point midway between references $j = 0$ and $1$ ($w = \tfrac12$) has $R' = R_i - \tfrac12 R_{ref,1} + \tfrac12 R_{ref,0}$. So

$$u^2(R') = u_i^2 + \tfrac14 u_{ref,1}^2 + \tfrac14 u_{ref,0}^2 + u_{interp,i}^2$$

With the typical 15–30 µΩ, a point's uncertainty grows by roughly a third. The fit's statistical uncertainty in R₀ also gains the first reference's uncertainty, since every corrected point includes $R_{ref,0}$. The covariance matrix accounts for all of that.

### The corner analysis

The readback-offset corner analysis (3.4.5) recomputes every resistance with currents shifted by $\pm\delta_{max}$. The references' resistances depend on the offset too, so they're recomputed with the same shift, and the correction applied afresh, before each corner's refit. That's why the fit receives the references' $V$ and $I$, not just their resistances.

### What R₀ and R₂₀ now refer to

- **R₀** describes the filament with its mount at its temperature at $t_0$, the first reference. If the filament has rested before the run (see "Starting cold"), that's the flange temperature.
- **R₂₀:** the correction to 20 °C (3.4.7) should therefore use the **start** chamber temperature as $T_f$, not the mean of start and end. Any warming of the chamber during the run reaches the filament through its mount, so the references see it and the correction removes it.
  - $a_T$ becomes `THERMOMETER_BOUND_KELVIN`, without the half-drift term.
  - The end temperature is still asked for and recorded, as a diagnostic.

## The schedule

**The reference current, `REFERENCE_CURRENT_AMPS`, is 200 mA.**
- It's in the middle of the setpoints, so it gives a good voltage (about 20 mV) without adding as much heat as 300 mA would.
- It isn't one of the fit's points. 200 mA is still measured separately as a setpoint, in its shuffled place.

**When references are measured:**
- **first**, before any setpoint;
- **after the first setpoint,** where the drift changes fastest;
- **then after every second setpoint;**
- **last**, after the final setpoint.

With nine setpoints the order is:

```
ref, p1, ref, p2, p3, ref, p4, p5, ref, p6, p7, ref, p8, p9, ref
```

That's six references and fifteen measurements, about 10 minutes instead of about 6.

The schedule comes from a pure function, `measurement_plan(setpoints) -> Vec<MeasurementType>` with `MeasurementType::{Reference, Setpoint(f64)}`, so it can be unit tested. Its two parameters are constants: `REFERENCE_CURRENT_AMPS`, and `REFERENCE_INTERVAL` (2 setpoints).

**Polarity alternation (3.2.2.1) carries on through the references.** Each reference starts in whichever polarity the relays are in and ends in the other, as a setpoint does.

## Starting cold

R₀ refers to the mount's temperature at the first reference. For that to be the flange temperature, the filament must have been unpowered for long enough beforehand, 6τ (about 15 minutes) for 99.75% of any earlier warming to have gone.

- **A new confirmation in `prepare`:** "Confirm that the filament hasn't been powered for at least 15 minutes". A run straight after another would otherwise start with a warm mount. The procedure can't check this itself.
- **The scale search** (3.1) runs before the first reference and briefly takes the filament to 300 mA, for about 6 s of ramp up, 2–3 s at 300 mA and 6 s of ramp down. That warms the mount by an estimated 30 µΩ (0.03%). It's small enough to accept and leave uncorrected. If it ever matters, a pause at 0 A between the scale search and the first reference would remove it.

## Changes

### `procedure/characterisation.rs`

- **Recording times:**
  - `measure_cold_resistance` takes `let start = tokio::time::Instant::now()` at its start.
  - `take_samples` returns the midpoint of its sampling window as seconds since `start`, as well as the samples.
  - `measure_polarity` passes it on, in a new `PolarityMeasurement::time_seconds`.
  - It's tokio's `Instant`, so the paused-clock end-to-end test gets consistent times.
- **The setpoint loop** (3.2) follows `measurement_plan`.
  - A reference is measured exactly like a setpoint, by `measure_setpoint`, including its per-point analysis. It's recorded in `ColdResistance::references` rather than `points`, through a `MeasurementSlot` argument: `Points` or `References`.
  - Each reference shows its resistance and its drift since the first reference as measurement steps, e.g. "Reference: 98.33 mΩ (+0.12 mΩ since the start)". The drift display is only a difference, so it's fine in Rust.
- **The fit** (`fit_setpoints`) passes the references, and every point's and reference's time, to `cold_resistance_fit.py`.
- **The results display** (3.5) adds the drift over the run, from the fit's output.
- **`fit_parameters`** uses the start chamber temperature for $T_f$, and the thermometer bound alone for $a_T$ (see "What R₀ and R₂₀ now refer to").
- **`prepare`** gains the confirmation in "Starting cold".
- **New constants:** `REFERENCE_CURRENT_AMPS` (0.2) and `REFERENCE_INTERVAL` (2), each with a doc comment.

### `results.rs`

- **`ColdResistancePoint`** gains `negative_time_seconds` and `positive_time_seconds`: the midpoints of each polarity's sampling window, in seconds since the cold-resistance measurement started.
- **`ColdResistance`** gains `references: Vec<ColdResistancePoint>`, in the order measured.
- **`ColdResistanceAnalysis`** gains:
  - `corrected_resistances_ohms: Vec<Derived>`: each point's $R'_i$ with its full uncertainty, in the order of `points`. That's what the visualiser plots once there's a fit.
  - `drift_corrections_ohms: Vec<Derived>`: each point's $\Delta R(t_i)$, for diagnosis.
  - `reference_drift_ohms: Derived`: the last reference less the first, which is the drift over the run.

Results files from before this change aren't supported.

### `python.rs` and the schemas

- **`ColdResistanceFitPoint`** gains `time_seconds: f64`, the mean of the point's two polarity times.
- **`ColdResistanceFitInput`** gains `references: Vec<ColdResistanceFitPoint>`. A reference is passed like a point: its reversal-corrected $V$ and $I$ from its own `ColdResistancePointAnalysis`, and its time.
- **Regenerate:**
  - the Python schemas, with `UPDATE_SCHEMAS=1` for `schemas_are_up_to_date`;
  - the results schema, with `results_schema_is_up_to_date`;
  - the visualiser's generated types, with `pnpm generate`.

### `cold_resistance_fit.py`

- `to_points(points, references, offset)`:
  - builds every point's and reference's $V$ and $I - \text{offset}$ as `ufloat`s;
  - computes the corrected $R'_i$ and $x_i = I_i^2$;
  - adds $u_{interp,i}$;
  - returns $x$, its uncertainties, the $R'_i$, and their covariance matrix.
- `fit_line` takes the covariance matrix as `sigma`. It checks that the matrix is symmetric and positive definite, replacing the current check that every uncertainty is positive.
- `offset_corners` passes the shifted offset through `to_points`, so the references are recomputed too (see "The corner analysis").
- **The uncertainty in $x$ warning** (3.4.1) compares $|b|\,u(x_i)$ with $\sqrt{C_{ii}}$.
- **New outputs:** `corrected_resistances_ohms`, `drift_corrections_ohms` and `reference_drift_ohms`.
- **New errors:**
  - fewer than two references;
  - a point outside the references' time range;
  - references whose times aren't increasing.
- **No drift warning.** The case worth catching is a filament that wasn't at the flange's temperature at the start, but that gives a smaller drift than usual, which the data alone can't tell from a normal run. The operator sees the drift after each reference and over the whole run, and judges whether the run is valid.

### Display and visualiser

- **The procedure's per-point display** stays uncorrected. A point's correction needs the reference after it, which doesn't exist yet when the point is shown.
- **The visualiser's graph** (`sections/cold-resistance.ts`):
  - Once there's an analysis, it plots the **corrected** points, with their full uncertainties, against the fitted line. The uncorrected points stay as faint markers, so the size of each correction is visible.
  - Before the fit, it plots the uncorrected points, as now.
  - **Later:** a second section plotting the references' resistance against time, which is the mount's warming curve, would be useful. It isn't part of this change.
- **The visualiser's fixtures** are recaptured from `--mock` runs, since the format changes.

### `2_COLD_RESISTANCE.md`

Update it to match:
- 3.2's measurement order;
- 3.4's correction;
- 3.4.7's $T_f$ and $a_T$;
- the results listing;
- the Python inputs and outputs;
- the "Readback noise, ripple and regulation drift" and "Incomplete thermal settling" entries in the error-sources appendix, adding the mount's warming as a source with how it's handled.

## Tests

### Python (`test_cold_resistance.py`)

1. **No drift:** with every reference equal, the corrected resistances equal the uncorrected ones, and R₀ and b match the uncorrected fit to $10^{-9}$ relative.
2. **Linear drift:** the existing fixture with $+d\,t$ added to every point and every reference, at times following the schedule. The fit must recover $R_0 = 0.096000$ Ω and the fixture's $b$, with $\chi^2 \approx 0$, because linear interpolation is exact for a straight-line drift.
3. **Exponential drift:** the fixture with $A(1 - e^{-t/\tau})$ added to points and references, with $A = 400$ µΩ, τ = 155 s, at the schedule's times.
   - R₀ is recovered to within the interpolation error;
   - χ²_ν is close to 1 when the fixture's noise is added;
   - each point's bound, $e_i = \sqrt3\,u_{interp,i}$, is at least the actual interpolation error at that point, for τ of 60, 95, 155 and 250 s.
4. **Propagation:** a point midway between references 0 and 1 has $u^2(R') = u_i^2 + \tfrac14 u_{ref,1}^2 + \tfrac14 u_{ref,0}^2 + u_{interp}^2$, matching the covariance matrix's diagonal to $10^{-12}$ relative. Two points $a$ and $b$ both between references 0 and 1 have the off-diagonal covariance $w_a w_b\,(u_{ref,0}^2 + u_{ref,1}^2)$, and two points both between references 1 and 2 have $(1 - w_a)(1 - w_b)\,u_{ref,1}^2 + w_a w_b\,u_{ref,2}^2 + u_{ref,0}^2$.
5. **Generalised least squares:** with a known covariance matrix, `fit_line`'s R₀, $b$, their uncertainties and χ² match the closed-form result, $\hat\beta = (X^\top C^{-1}X)^{-1}X^\top C^{-1}y$ with covariance $(X^\top C^{-1}X)^{-1}$, computed in the test, to $10^{-5}$ relative. This replaces test 6 in `2_COLD_RESISTANCE.md`, which is the diagonal case.
6. **Corner analysis:** the references' resistances change with the offset shift, and the corners match a hand-built correction at $\pm\delta_{max}$.
7. **Errors:** fewer than two references, a point outside the references' times, and references with times that aren't increasing each exit with a clear message.
8. **Schemas:** the existing schema tests with the new input and output fields.
9. **Existing tests:** they still pass, with the fixture's references added to their inputs, without drift. The exception is the offset corners: the shifted points don't lie on a line, so the corners depend on the weights, and those now include the references' uncertainties. The fixture's corners move from 0.092317 and 0.099849 Ω to 0.092013 and 0.100195 Ω, and its readback-offset uncertainty from 2.174 to 2.362 mΩ.

### Rust

- **`measurement_plan`:**
  - a reference comes first, after the first setpoint, after every second setpoint after that, and last;
  - there's never a doubled reference, e.g. when the last setpoint already falls at a reference position;
  - every setpoint has a reference before and after it;
  - the plan contains every setpoint exactly once.
- **Times:** on the paused clock, `take_samples`' midpoint is between its first and last sample.
- **The end-to-end tests, with and without vacuum:**
  - the results have six references;
  - every point and reference has increasing times;
  - every point's time lies between the first and last reference;
  - R₀ is still 0.096 Ω, because the mock has no drift, so the corrections are zero;
  - `reference_drift_ohms` is 0.

### Visualiser

- The corrected points are plotted once there's an analysis, and the uncorrected ones before. The fixtures cover both.
- The fixtures validate against the regenerated schema.

### On the rig

- **The precision resistor** (`--no-vacuum`): it has almost no temperature coefficient, so `reference_drift_ohms` should be within a few times the references' uncertainties of 0, and the corrections negligible.
- **The filament:** a run should give χ²_ν around 1–3 rather than 60–75, a `reference_drift_ohms` of about +0.3–0.5 mΩ, and an R₀ within about 0.2 mΩ of the uncorrected fit. Two runs a few hours apart, each started cold, should give R₂₀ within their statistical uncertainties of each other.

## Implementation order

Each commit builds, passes `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, the Python tests and the visualiser's checks, and runs end to end with `--mock`. Anything unused before the commit that uses it is marked `#[expect(dead_code)]`, naming that commit.

1. **Record when each polarity was measured.**
   - `take_samples` and `PolarityMeasurement` return the time, and `ColdResistancePoint` stores the two times.
   - Regenerate the results schema and the visualiser's types.
   - No behaviour change.
2. **Measure references.**
   - `measurement_plan`, its tests, `REFERENCE_CURRENT_AMPS` and `REFERENCE_INTERVAL`.
   - `MeasurementSlot` and `ColdResistance::references`, and the per-reference display.
   - The "Starting cold" confirmation.
   - The end-to-end tests check the references.
   - The fit is still uncorrected.
3. **Correct the fit.**
   - The fit's input and output types, regenerated schemas, and the script's correction, covariance and generalised least squares, with Python tests 1–9.
   - `fit_setpoints` passes the references and times, and shows the drift.
   - $T_f$ and $a_T$ from the start temperature.
4. **Plot the corrected points** in the visualiser, and recapture its fixtures.
5. **Update `2_COLD_RESISTANCE.md`** and the error-sources appendix.

## Alternatives considered

- **Resting between setpoints:** each point would start with a cold mount, and its own warming during the measurement would behave like self-heating, which the extrapolation removes. But with τ ≈ 155 s, a rest of about 4τ, roughly 10 minutes, per setpoint would make a run take 1.5–2 hours.
- **Warming up to equilibrium first:** it removes the scatter, but R₀ then describes a warm mount, about +0.4–1% high depending on the warm-up current. The error is systematic and not in the budget.
- **A thermal model:**
  - predict the mount's temperature from the power history, using a first-order response with the hold test's τ and gain of about 0.1 mΩ per mW, and fit it alongside R₀ and b;
  - it costs no extra measuring time, but the result depends on the model, and the hold test shows a second, slower component the model would miss.

  It remains a good cross-check: the predicted drift should agree with the references'.
- **Lower currents:** the drift scales with power, but the readback-offset uncertainty grows as the current falls, and it's already the largest term.
- **Fitting a smooth curve to the references,** e.g. an exponential approach, instead of interpolating linearly. That would reduce the interpolation error and give a measure of how well a point repeats. But it assumes the drift's shape, which the hold test shows isn't a single exponential. Consider it once more runs show whether the shape is consistent.
- **Including references in the fit's points:** after correction, every reference equals $R_{ref,0}$, so they add almost nothing about the line, while correlating with every corrected point. They're kept separate.

## Limitations and open questions

- **Remaining scatter:** with the drift removed, χ²_ν will probably still be about 2–3, because each point's uncertainty only covers its own 8 s of noise, not how well it repeats. The Birge inflation (3.4.2) covers that, as now. Estimating repeatability properly needs repeat measurements; the references don't give it, because linear interpolation passes exactly through them.
- **What the correction assumes:**
  - **That the drift is uniform along the filament.** A change that isn't uniform, e.g. one leg warming more than the other, still gives approximately the right correction to first order, but not exactly.
  - **That anything common to every reading is additive.** The references also see any additive instrument drift, which the correction removes too. A drift in the scope's gain would scale with the voltage instead, and would only be approximately removed. The resistor run showed no instrument drift.
- **The reference interval:** an interval of 2 setpoints, with an extra reference early, is a judgement based on τ ≈ 155 s. If runs show $u_{interp}$ dominating the per-point uncertainties, shorten it. If it's negligible, lengthen it to 3 to save time.
- **The reference current:** 200 mA trades signal against heating. 300 mA gives a 50% larger voltage, but adds its 9 mW to every reference.
- **Readback offset:** this improves R₀ by about 0.1–0.4%, while the readback offset contributes about 2.3%. It's worth doing for b, for repeatability between runs, and for the heating characterisation. For R₀'s accuracy, calibrating the readback offset matters far more.
- **The heating characterisation:** the same references, measured at a fixed low current between hot setpoints, would track the mount there too. Its warming is far larger at watts than at milliwatts, so this machinery is worth designing to be reused.
