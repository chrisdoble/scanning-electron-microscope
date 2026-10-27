# Pulse heating

Oct 26, 2026 · @Chris

The specification for the pulse binary.

This document describes a new binary in the `characteriser` crate. It drives the filament with a short pulse of high current, e.g. 2 A for 5 s, and measures how much the pulse warmed the mount. It builds on `1_ARCHITECTURE.md`, `5_HOT_SWEEP.md` and the analysis in `MOUNT_WARMING_ANALYSIS.md`.

Unlike the characteriser, it's a plain command-line program: no terminal UI, just lines printed to stdout. It doesn't control or monitor the TMP. It only checks at startup that the chamber pressure is low enough to power the filament, and exits with an error if it isn't.

## Why

The hot sweeps showed that the mount can't take the currents needed to make the filament incandescent for long. It warms by about 0.45 K per mW of steady filament power, with a time constant of about 2 minutes, and the epoxy softens at about 80 °C. But a pulse much shorter than the mount's time constant puts in far less heat than the same current held, so a few seconds of glow should be possible. The binary has two goals:

1. **Make the filament glow on camera,** for a few seconds, without the epoxy softening.
2. **Measure how much each pulse warmed the mount,** so the current and duration can be raised step by step, each step predicted from the last.

The second goal is what makes the first safe: every pulse is also a measurement that says whether the next, larger pulse is acceptable.

**Not in scope:**

- **The filament's temperature.** The hot resistance during the pulse is recorded, but converting it to a temperature isn't attempted, for the same reasons as in `5_HOT_SWEEP.md`.
- **Holding the filament incandescent.** That needs a rebuilt mount.
- **More than one pulse per run.** Each run fires one pulse and logs the mount cooling afterwards, which takes about 10 minutes anyway. Repeating is a matter of running again.
- **Capturing the voltage waveform during the pulse.** The scope only reports each acquisition's average, maximum and minimum; the waveform will be captured separately when characterising the time constants.
- **The vacuum system.** Pumping down, the TMP and spinning down are left to the operator, e.g. with the characteriser. The pressure is only read, not monitored.

## What to expect

These numbers set the scale of everything below. They're estimates, and the binary exists to replace them with measurements.

**Currents.** The 1D wire model from the earlier analysis, for tungsten–rhenium wire with conduction to the posts and radiation, gives:

- **dull red,** visible in a dark room, at about 1.4 A, almost independent of the wire's diameter;
- **about 2700 K,** bright incandescence, at 1.98, 2.46 or 3.08 A for 100, 125 or 150 µm wire. The diameter isn't known yet.

The supply's maximum is 3 A. For filming, a bright orange glow at somewhere around 1.6–2.0 A may be enough, and takes much less power than white heat.

**Power during a pulse.** The hot filament's resistance is several times its cold 98 mΩ, so P = I²R is likely to be a few watts at 2 A. That's 50–100 times the sweep's 39–78 mW.

**How much a pulse warms the mount, if the sweep's model held.** For a pulse much shorter than τ, the model gives X_end ≈ k·E/τ, where E is the pulse's energy. With k/τ from both sweep fits, about 1.1 mΩ per joule:

|                      | Per joule | 5 J    | 10 J  | 20 J  |
| -------------------- | --------- | ------ | ----- | ----- |
| Extra resistance     | 1.1 mΩ    | 5.5 mΩ | 11 mΩ | 22 mΩ |
| Clamp rise, W-Re α   | 3.7 K     | 18 K   | 37 K  | 73 K  |
| Clamp rise, pure W α | 2.5 K     | 13 K   | 25 K  | 50 K  |

So 2.5 W for 5 s, 12.5 J, would be about 45 K with the W-Re α, which is too much. Short pulses are the way in.

**Why this probably overestimates the warming.** In the sweep the wire stayed cool, so nearly all its power was conducted to the mount. An incandescent wire radiates most of its power into the chamber, and only what's conducted along its legs reaches the clamp, which is limited by the legs' thin cross-section. So a joule of pulse energy should warm the mount less than a joule in the sweep did, though some of the radiated power will land on the posts. The measurement decides.

**Why it might underestimate the warming locally.** The sweep's model lumps the mount into one 2-minute time constant. A 5 s pulse delivers its heat to the clamp faster than it can spread, so the clamp, and any epoxy right next to it, peaks higher than the lumped value before the heat spreads. Nothing the probe current measures afterwards can see faster than a few seconds (see "What the measurement can and can't see").

## Changes outside the binary

### A library target

The characteriser is currently a single binary: every module is declared in `main.rs`. A second binary can only share code through a library. The pulse binary needs the hardware, not the terminal UI or the procedure, so the library holds only what's independent of them:

- **`src/lib.rs`** declares `constants`, `hardware`, `python` and `results`, which don't depend on the UI or the procedure.
- **`build_hardware`** moves from `main.rs` into `hardware`, so both binaries build the hardware from the same connection options (only the characteriser keeps its mock mode):
  - the device path, the ADC gauge number and the TMP address, positional arguments in both, for the controller, the gauge and the TMP.
- **`src/main.rs`** keeps `app`, `procedure`, `steps`, `style` and `ui` as its own modules, and uses the rest from the library.
- **`src/bin/pulse.rs`** is the new binary, in one file unless it grows.

The pulse binary needs the TMP's address only because `RealVacuumSystem` is built with it. It never commands the TMP. Nothing changes in behaviour for the characteriser.

### Sampling helpers

The measurement helpers in `procedure/characterisation/measurement.rs` take a `Context`, which belongs to the terminal UI, for its step tree and its once-a-second snapshots. The pulse binary can't use those.

The parts that don't need a `Context` move into a library module, `sampling`, and both binaries use them:

- `unclipped_average`, `mean_and_standard_error`;
- `VERTICAL_SCALES_VOLTS_PER_DIVISION`, `CLIPPING_HEADROOM`, `OVERCURRENT_PROTECTION_MARGIN_AMPS` and `RELAY_SETTLE_TIME`.

The pulse binary has its own short versions of `change_polarity` and `sample` that print instead of updating the UI, and don't check the chamber. It doesn't ramp: every current change is a jump, so it needs no `ramp_to`.

### Results

A new top-level type, `PulseRun`, defined in `src/bin/pulse.rs` rather than in the library's `results.rs`, so pulse-specific code stays out of the library. It's its own type rather than a section of `Characterisation`, since the binary has no `Context` to record into. It's saved as `out/pulse-<started_at>.json` once at the end, and also on every early exit with whatever was collected (see "Exiting safely"). It derives `JsonSchema` like the other results, so the analysis script's input can be validated in the same way. Its fields are under "The results file".

## The binary's arguments

```
pulse <DEVICE_PATH> <ADC_GAUGE_NUMBER> <TMP_ADDRESS> --filament-id 1 --current 2.0 --duration 5 --voltage-limit 1.0 [--maximum-energy 10]
```

| Argument                                         | Meaning                                                               | Default  | Limits                                                                                        |
| ------------------------------------------------ | --------------------------------------------------------------------- | -------- | --------------------------------------------------------------------------------------------- |
| `--filament-id`                                  | Recorded in the results                                               | required |                                                                                               |
| `--current`                                      | The pulse current, in A                                               | required | above the probe current, at most `POWER_SUPPLY_MAXIMUM_CURRENT_AMPS`                          |
| `--duration`                                     | How long the pulse is held, in s                                      | required | more than 0, at most `MAXIMUM_PULSE_DURATION`                                                 |
| `--maximum-energy`                               | Ends the pulse early once it has delivered this much energy, in J     | none     | more than 0                                                                                   |
| `--voltage-limit`                                | The supply's voltage limit throughout, in V (see "The scope's scale") | required | above the pulse current through `COLD_RESISTANCE_OHMS`, at most `MAXIMUM_VOLTAGE_LIMIT_VOLTS` |
| `DEVICE_PATH`, `ADC_GAUGE_NUMBER`, `TMP_ADDRESS` | Positional, as for the characteriser                                  |          |                                                                                               |

Out-of-range arguments are rejected before anything is powered.

## Constants

| Constant                                 | Value  | Why                                                                                                                                                                                                                                                                                                                                  |
| ---------------------------------------- | ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `MAXIMUM_PULSE_DURATION`                 | 10 s   | Longer than filming needs. A pulse this long at incandescent currents would put tens of joules into the mount.                                                                                                                                                                                                                       |
| `MAXIMUM_VOLTAGE_LIMIT_VOLTS`            | 5.0    | The highest `--voltage-limit` allowed. Bounds the pulse's power. Above what 3 A needs unless the hot resistance exceeds 1.7 Ω.                                                                                                                                                                                                       |
| `PROBE_CURRENT_AMPS`                     | 0.1    | Keeps the probe's own warming negligible (see "Why 100 mA").                                                                                                                                                                                                                                                                         |
| `BASELINE_DURATION`                      | 60 s   | Long enough to average the probe's noise down to about 0.1 K, and to check that the mount isn't still cooling.                                                                                                                                                                                                                       |
| `COOL_DOWN_DURATION`                     | 600 s  | About five of the mount's 2-minute time constants, so the fit sees the slow decay through to the baseline.                                                                                                                                                                                                                           |
| `BASELINE_MAXIMUM_SLOPE_OHMS_PER_SECOND` | 3 µΩ/s | Rejects a baseline taken while the mount is still cooling. A mount whose resistance is X above its cold value, cooling with τ ≈ 2 minutes, drifts at X/τ. So 3 µΩ/s means X ≈ 0.36 mΩ, about 1 K warm. A tighter limit would trip on noise: the fitted slope over 60 s is uncertain by about this much.                              |
| `REVERSAL_BLOCK_DURATION`                | 10 s   | How long the probe stays in each polarity before switching, about 25 samples (see "The probe measurement").                                                                                                                                                                                                                          |
| `FIT_SKIP`                               | 15 s   | Cool-down samples this soon after the pulse are recorded but left out of the fit and the printed progress, while the filament itself cools. After the first 1 A pulse, the filament's tail was still about 1 mΩ 5–8 s after the pulse, several times the mount's warming; with a 5 s skip it inflated the fitted rise about twofold. |
| `CLEANUP_TIMEOUT`                        | 5 s    | As in the characteriser: how long making the filament safe may take before it's given up on.                                                                                                                                                                                                                                         |
| `CLAMP_RISE_BUDGET_KELVIN`               | 25 K   | In pulse.py, not the binary. Only used to print the energy that would reach it; nothing enforces it (see "The budget").                                                                                                                                                                                                              |

## The procedure

Every step prints a line saying what it's doing, and every measurement prints its result.

### 0. Startup

- **Check the arguments.**
- **Check the Python environment** with `python::check_environment`, as the characteriser does, so a broken environment is reported before anything is powered rather than at the end.
- **Build the hardware.**
- **Check the pressure.** Read the vacuum system's snapshot once. If the pressure isn't below `FILAMENT_OPERATING_PRESSURE_MBAR`, exit with an error saying what it is and what it needs to be. The TMP's state isn't checked: a low pressure is what the filament needs, however it was reached.

There are no confirmations as in the characteriser's `prepare`. The baseline's slope check stands in for confirming that the filament hasn't been powered for 15 minutes.

### 1. Setup

- **Supply:**
  - voltage limit `--voltage-limit`;
  - overcurrent protection at the pulse current plus `OVERCURRENT_PROTECTION_MARGIN_AMPS`;
  - current 0, output on, forward polarity.
- **Jump to the probe current,** as for the pulse: the supply is constant-current, so there's no inrush.
- **Scope:** with the probe current flowing, the scale for it, chosen by stepping up from the smallest until a reading doesn't clip, as `read_unclipped` does.

### 2. Baseline

Record the probe resistance into the PulseRun results for `BASELINE_DURATION` (see "The probe measurement"), printing the elapsed time and the mean resistance so far every 10 s, so it's clear the run is still going.

- **Baseline resistance:** the mean of the baseline samples.
- **Slope:** a straight-line fit to the baseline samples against time.

Both are computed in the binary as soon as the baseline ends, not by the analysis script, and printed. If the slope's magnitude is more than `BASELINE_MAXIMUM_SLOPE_OHMS_PER_SECOND`, the mount is probably still cooling from something earlier. The run exits with an error saying so, and the operator waits and runs it again.

### 3. Fire

- **Check the pressure again.** Read it once more, since the baseline took a minute, and exit with an error if it's no longer low enough.
- **Set the pulse scale before the pulse.** It comes from the voltage limit, not a measurement (see "The scope's scale"). Changing the scale takes a second or two, as the clamping fix showed, so it can't happen during the pulse (see "The pulse"). The pulse follows straight after, with no prompt or countdown: the camera is started before the binary is run.

### 4. The pulse

- **Jump, don't ramp.** Set the current limit straight to the pulse current. The supply is constant-current, so there's no inrush, and ramping at 50 mA/s would take almost 40 s to reach 2 A, putting more heat into the mount than the pulse.
- **Sample throughout.** Take voltage and current samples, timestamped individually, as fast as the instruments allow, about 0.4 s each. That's about 12 samples in 5 s. Each sample checks the overcurrent trip and the regulation mode, as `sample` does. Each sample's current, voltage and power are printed.
- **Timer-driven end.** The pulse ends when `--duration` has passed since the current was set, not after a number of samples. A sample in progress at the deadline is abandoned, so the overshoot is at most the one instrument call that's in flight, about 0.1 s. The times the current was set up and back down are recorded, and the actual duration is printed.
- **Energy.** The pulse's energy is accumulated from the samples, E = Σ V·I·Δt. With `--maximum-energy`, the pulse ends early once E passes it, and that's recorded.
- **Jump back** to the probe current at the end.

**The scope's scale.** The pulse's scale is the smallest that fits `--voltage-limit` with `CLIPPING_HEADROOM`, e.g. 0.5 / (4 × 0.8) = 0.16, so 0.2 V/div. The supply can't put more than the voltage limit across the filament, so the reading can't clip. So the voltage limit should be about twice the expected pulse voltage, I·R_hot: much higher and the reading is coarse. The first 1 A pulse ran with a fixed 5 V limit, so 2 V/div, and its 0.13–0.16 V readings were about a tenth of a division: a step of the 8-bit scope is about 60 mV there, and the offset error is likely tens of mV, so its energy may be off by 20% or more. A limit near the expected voltage also bounds the pulse: a hot resistance higher than expected ends it (see "Leaving constant current").

**Leaving constant current.** If the filament's hot resistance is more than the voltage limit divided by the current, the supply goes into constant voltage, which caps the power at about V_limit·I. That's a safety limit working, not a fault, but it also happens when the circuit opens, e.g. if the filament burns out. Either way the pulse ends, and the reason is recorded. A current readback near zero is reported as an open circuit.

**The pressure isn't monitored during the pulse.** Reading the gauge would slow the sampling, and the pulse lasts seconds. A filament glowing for the first time outgasses, so the pressure may rise briefly. The next run's startup check catches a chamber that hasn't recovered.

### 5. Cool-down

- **Reset the scale.** Set the probe's scale again. Its first second or two of samples are lost, but `FIT_SKIP` excludes them from the fit anyway.
- **Tolerate clipping within `FIT_SKIP`.** Right after the pulse, the current is still falling to the probe current (the supply takes about 0.6 s to apply a setpoint) and the filament is hot, so the first samples can clip on the probe's scale; after the 1.5 A pulse the first read up to 43 mV, past the 32 mV that 10 mV/div allows. Clipped samples before `FIT_SKIP` are dropped and counted in a warning. After it, clipping is an error as everywhere else.
- **Record** the probe resistance for `COOL_DOWN_DURATION`, timestamped, into the `PulseRun` results.
- **Print it as it goes.** Every 10 s, print the time since the pulse, the extra resistance against the baseline, and the clamp rise it implies with the W-Re α, from the latest pair of blocks after `FIT_SKIP`. Each value is a model-free lower bound on the warming at that moment. Without the skip, the first pair includes the filament's own cooling: after the first 1 A pulse it read +7 K when the mount was about 0.7 K warm.

### 6. Analyse and finish

- **Make the filament safe:** current to 0, output off, relays de-energised, with `enter_safe_state`.
- **Save the results.**
- **Analyse:** run the analysis (see "Analysis") through `python.rs`, add its results to the file, save again, and print a summary:
  - the pulse's duration and energy;
  - the extrapolated extra resistance and clamp rise at the end of the pulse, with both αs;
  - K per joule;
  - the energy that would reach the budget, CLAMP_RISE_BUDGET_KELVIN.

### Exiting safely

The characteriser's careful shutdown in `main.rs` exists because of the terminal UI and the procedure task. The pulse binary is a single sequence, so it's simpler, but the rule is the same: **every exit path puts the filament system into a safe state.**

- **Normal completion and errors:** the sequence runs inside a function whose result is handled in one place, which always calls `enter_safe_state` before reporting.
- **Ctrl-C:** the sequence is raced against `tokio::signal::ctrl_c()`. If Ctrl-C wins, the sequence is dropped and the same cleanup runs. That matters most during the pulse and the 10-minute cool-down.
- **Panics:** the sequence is wrapped in `catch_unwind`, as the characteriser's procedure task is, so a panic also reaches the cleanup.
- **A hung instrument:** the cleanup is bounded by `CLEANUP_TIMEOUT`. If it fails or times out, print a clear warning that the filament may still be powered.
- **Results are saved on every path,** with whatever was collected, so a run that stops mid-cool-down still records its pulse.

## The probe measurement

**Why 100 mA.** The probe current's own power warms the mount too:

- 100 mA puts in about 1 mW, which would settle at about 0.14 mΩ, or 0.45 K. That's small enough to ignore.
- 300 mA puts in 10 mW, which would settle at about 4.6 K, taking about 10 minutes to do so. That would drift across the baseline and the cool-down.

The cost of 100 mA is noise. In the sweeps, a single 100 mA sample scattered by about 0.28 mΩ, or 0.9 K, but averaging takes it down quickly:

- the 60 s baseline's mean is good to about 25 µΩ, or 0.1 K;
- the cool-down fit's amplitude is good to a few tens of µΩ.

**Polarity reversal.** The sweep's 100 mA resistances were 2.5–4 mΩ low, from a residual voltage of about −0.3 mV that varied between runs. A constant offset would cancel between the baseline and the cool-down, but this one could change _because of_ the pulse. The pulse heats the joints between dissimilar metals at the clamps, and any imbalance between the two ends changes the thermal EMF. A 10 µV change is 0.1 mΩ at 100 mA, or about 0.3 K.

Reversing the polarity cancels both offsets and thermal EMFs, as in the cold resistance:

- **When:** probe samples alternate between forward and reverse polarity in blocks of `REVERSAL_BLOCK_DURATION` (10 s). The offset only needs tracking as fast as it changes, and it follows the clamp's temperature, which changes over minutes. Over 10 s it should move by a few µV at most, about 0.03 K.
- **How:** each reversal turns the output off for `RELAY_SETTLE_TIME` (50 ms) while the relays switch. At 100 mA that's harmless.
- **Combining:** each block is averaged to one V, I and mean time t. Every neighbouring pair of blocks, one forward and one reverse, gives R = (V₊ − V₋)/(I₊ + I₋), with V₋ negative, timestamped at the midpoint of the two blocks' times. A changing R is then measured at that midpoint. An offset drifting by ΔE between the two blocks adds ΔE/(I₊ + I₋), with the opposite sign for the next pair, so it averages out across the fit.
- **Relay wear:** about 6 reversals in the baseline and 60 in a 10-minute cool-down.

The hot sweep didn't reverse because its voltages were hundreds of millivolts, so a fraction of a millivolt didn't matter. At the probe current the voltage is only about 10 mV, so it does.

The pulse itself is always in forward polarity, and its samples aren't reversed: the voltage is volts, and only the power matters.

## Analysis

A new script, `characteriser/python/pulse.py`, run by the binary at the end through `python.rs`, with an input schema like the other scripts. From the samples it works out:

**The pulse:**

- the actual duration;
- the energy E, and the mean and peak power;
- the hot resistance at each sample, and its last value.

**The baseline,** recomputed from the saved samples, with uncertainties. The go/no-go check on the slope has already run in the binary, in "2. Baseline":

- the mean resistance, R_b, with its uncertainty;
- the slope, with its uncertainty.

**The warming:**

- **First reading:** the extra resistance at the first cool-down point after `FIT_SKIP`, against R_b. It's model-free and a lower bound.
- **At the end of the pulse:** a fit to the cool-down points of R(t) = R_b + A·e^(−t/τ), with t measured from the end of the pulse. A is the extra resistance at the end of the pulse, X_end.
  - One time constant to start. The sweeps and the hold test hint at a slower component, so a second term can be added if the residuals show one.
  - The baseline points are fitted along with the cool-down, as R = R_b, so the two share R_b and its uncertainty carries into A.
  - Neighbouring probe resistances share a block, so their noise is correlated, about 0.5. The fit weights each point by its own variance only, because a fit using the full covariance would weight heavily the pattern that alternates from pair to pair, which is what a drifting offset leaves. The correlation goes into the uncertainties instead, through the sandwich covariance. On synthetic runs at the rig's noise, A's errors spread by 0.96 of its stated uncertainty, and A is good to about 40 µΩ, 0.13 K.
- **Clamp rise:** X_end / (R₀α), with both αs, as in `MOUNT_WARMING_ANALYSIS.md`.

**Per joule:**

- **K per joule:** the clamp rise divided by E. This is the number used to plan the next pulse.

**For the next pulse:** the energy that would give the budget's clamp rise at the measured K per joule (see "Using it safely").

## The results file

`PulseRun`, saved as `out/pulse-<started_at>.json`:

| Field                       | Contents                                                                                                                                                  |
| --------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `filament_id`, `started_at` | as in `Characterisation`                                                                                                                                  |
| `parameters`                | the arguments as given, and the constants in force                                                                                                        |
| `pressure_mbar`             | the pressure at startup and just before firing                                                                                                            |
| `scales`                    | the probe's and the pulse's vertical scales                                                                                                               |
| `baseline`                  | the baseline samples: time, polarity, voltage, current                                                                                                    |
| `pulse`                     | the pulse samples: time, voltage, current; the times the current was set up and back down; how the pulse ended (duration, energy limit, constant voltage) |
| `cool_down`                 | the cool-down samples, as for the baseline                                                                                                                |
| `analysis`                  | the analysis's results, as above                                                                                                                          |
| `warnings`                  | e.g. a clipped reading                                                                                                                                    |
| `error`                     | why the run stopped early, if it did                                                                                                                      |

Times are in seconds since the run started, as `time_seconds` is elsewhere.

## What the measurement can and can't see

- **It can see** the clamp's lumped temperature from `FIT_SKIP` after the pulse onwards, and, by extrapolation, its value at the end of the pulse.
- **It can't see** anything that settles faster than `FIT_SKIP`. That includes heat spreading out from the clamp in the first seconds, while the filament and its legs cool. The local peak at the clamp during the pulse, and in any epoxy within a millimetre or two of it, can be higher than X_end suggests.

So X_end is a lower bound on the hottest spot, and the budget leaves room for that.

**An independent check would help.** Irreversible temperature indicator labels, which change colour permanently at a stated temperature, stuck to the posts near the epoxy and read after each session, would show directly whether the hottest spot got near 80 °C. They might outgas, so they're best outside the vacuum, or on the outside of a post if the geometry allows.

## Using it safely

**The budget.** Only fire a pulse whose predicted extrapolated clamp rise, with the W-Re α, is at most 25 K, about 45 °C from 20 °C. That rise is ΔT = E_next × (K per joule), with K per joule = X_end / (R₀α) / E measured from the last pulse, where X_end is the cool-down fit extrapolated back to the end of the pulse. Before any pulse, the sweep's 1.1 mΩ per joule stands in for X_end/E. That leaves 35 K for the hotspot, the α uncertainty, and the room being warm. The number is a starting point to revise once the indicator labels, or a rebuilt mount, say more.

**Stepping up.**

1. **First pulse:** a pulse that's surely harmless and still tells us something, e.g. 1.0 A for 2 s. That's probably under 2 J, so a few K at most even with the sweep's model.
2. **After each pulse:**
   - **Predict the next pulse's energy.** A pulse's energy is close to E = P·t, where P = I²·R_hot is the steady power: the mean of V·I over the pulse samples after the first second. Below glow the filament may not quite reach its hot resistance within a 2 s pulse: at 1 A it was still rising at 1.7 s, from 134 to 154 to 161 mΩ. So P from a short pulse is a slight underestimate, and the 1.2× margin on `--maximum-energy` covers it. P grows faster than I² because R_hot rises with the temperature, so the question is how much faster. For a pulse at I_next lasting t_next:
     - **From one pulse** (I₁, P₁): E_next ≈ P₁·(I_next/I₁)³·t_next. For a glowing tungsten filament losing heat mostly by radiation, P ∝ T⁴ and R ∝ T^1.2 give P ∝ I^2.9, so the cube is a fair guess. Below glow, where conduction to the posts dominates, P can rise faster than I³, so treat this as rough.
     - **From two or more pulses:** use the local exponent from the two most recent, n = ln(P₂/P₁) / ln(I₂/I₁), then E_next ≈ P₂·(I_next/I₂)ⁿ·t_next. Only the recent pulses, because n changes as the filament moves from conduction to radiation.
     - **The voltage limit:** set `--voltage-limit` to about 2·I_next·R_hot, with R_hot extrapolated from the last pulse's final resistance. If R_hot turns out higher, the supply goes constant-voltage and the pulse ends.
     - Errors in n are what the 1.5× energy step below protects against: each step is checked by the next pulse's measured P before it's extrapolated again.
   - **Predict its clamp rise.** Multiply the predicted energy by the measured K per joule.
3. **Only fire the next pulse** if the predicted rise is within the budget, and the energy is at most 1.5 times the last pulse's. That limits how far each extrapolation goes.
4. **Use `--maximum-energy`** at the predicted energy times 1.2 or so, as a backstop for a hot resistance higher than expected.
5. **Raise the current first,** at a fixed short duration, until the glow is bright enough to film. Then lengthen the duration within the budget.

**Between pulses,** wait for the mount to cool to room temperature, about 15 minutes (6τ). The baseline's slope check catches a mount that hasn't cooled.

**The chamber stays pumped** between pulses, with the TMP running. Bring it down and back up with the characteriser, or by hand.

**A plan to start from.** Each step only goes ahead if the rule above allows it:

| Pulse | Current                     | Duration  | Expect                   |
| ----- | --------------------------- | --------- | ------------------------ |
| 1     | 1.0 A                       | 2 s       | no glow                  |
| 2     | 1.2 A                       | 2 s       | no glow                  |
| 3     | 1.4 A                       | 2 s       | dull red, in a dark room |
| 4     | 1.6 A                       | 2 s       | red to orange            |
| 5     | 1.8 A                       | 2 s       | orange                   |
| 6     | 2.0 A                       | 2 s       | bright orange to yellow  |
| 7+    | the current that looks good | 3, 4, 5 s |                          |

## Open questions

- **Is 25 K the right budget?** It depends on where the epoxy is relative to the clamp, and on how much hotter the clamp gets than X_end during a pulse. Both are unknown.

## Implementation order

1. **The library split:** `constants`, `hardware` with `build_hardware`, `python`, `results`, and the `sampling` module, with the characteriser unchanged. Check it still runs, with `--mock` and on the rig.
2. **The `pulse` binary's arguments, startup checks, sequence and safe exit,** logging only: no analysis, a fixed probe without reversal. Try it on the rig at a harmless current, e.g. 0.3 A for 2 s, including Ctrl-C at each stage.
3. **Polarity reversal for the probe.**
4. **The analysis script and its schema,** printing the summary at the end.
5. **The first real pulses,** following "Using it safely".

**Tests,** written alongside each step, for the safety-relevant logic, which is pure and cheap to test: argument validation, the pressure check, the pulse's timer-driven end, the energy limit, combining the reversed probe pairs, and choosing the pulse's scale. A full run against the mock hardware with a paused clock would also check that every exit path makes the filament safe, and may be worth adding.
