# Cold resistance calculation steps

This document specifies the real body of `measure_cold_resistance` in `filament-characterisation/characteriser`, plus the changes it needs elsewhere in the crate and in `host`. It builds on `docs/ARCHITECTURE.md`. Everything there still applies unless this document says otherwise: code style (§18), doc comments, saving after every measurement (§10.3), safe shutdown (§16), and "no statistics in Rust" (§2).

The filament is measured four-terminal, with current reversal, at a set of currents. The resistance at each current is plotted against $I^2$, and a weighted straight-line fit is extrapolated to $I = 0$ to give the cold resistance $R_0$ and its full uncertainty budget.

## Why extrapolating to $I^2 = 0$ gives $R_0$

Any current heats the filament, so every measured resistance is slightly above the cold resistance. At zero current there's no heating, but there's also no voltage to measure. The fit works around this by measuring where the signal is good and extrapolating back.

At each setpoint the filament settles where the electrical power going in equals the heat going out. Heat leaves in two ways.

**Conduction** along the wire into its mount follows Fourier's law: heat flows down a temperature gradient at a rate $q = -kA\,dT/dx$, where $k$ is the thermal conductivity and $A$ the cross-section. Take a length $L$ of wire carrying heat from the filament at $T$ to the mount at $T_0$, with no heat generated inside it. In steady state, whatever heat enters one end must leave the other, so $q$ is the same at every point. With $k$ and $A$ constant, $dT/dx = -q/(kA)$ is then constant too, so the temperature falls linearly from $T$ to $T_0$. That means $dT/dx = (T_0 - T)/L$, and

$$q = -kA\,\frac{T_0 - T}{L} = \frac{kA}{L}\,(T - T_0)$$

This is proportional to $\Delta T$.

The filament itself is heated along its whole length rather than at one end. With power $p$ per unit length and both ends held at $T_0$, the steady state is $kA\,d^2T/dx^2 = -p$. That gives a parabolic profile, $T(x) - T_0 = p\,x(L - x)/(2kA)$, whose mean rise is $pL^2/(12kA)$. So the total power is $P = pL = (12kA/L)\,\Delta T_{mean}$, still proportional to the temperature rise. Either way, the proportionality holds because the equations are linear as long as $k$ doesn't change over the small temperature range.

**Radiation** follows the Stefan–Boltzmann law: a surface at $T$ emits $\varepsilon\sigma A T^4$. It also absorbs from its surroundings at $T_0$, and for a small object inside a large enclosure it absorbs with the same efficiency that it emits (Kirchhoff's law), so it absorbs $\varepsilon\sigma A T_0^4$. The net loss is $\varepsilon\sigma A(T^4 - T_0^4)$. This isn't linear, but for a small rise it nearly is. Writing $T = T_0 + \Delta T$ and expanding to first order:

$$T^4 = T_0^4\left(1 + \frac{\Delta T}{T_0}\right)^4 \approx T_0^4\left(1 + \frac{4\Delta T}{T_0}\right) = T_0^4 + 4T_0^3\,\Delta T$$

That's just the derivative $d(T^4)/dT = 4T^3$. So the net loss is $\approx 4\varepsilon\sigma A T_0^3\,\Delta T$, and the first term dropped is smaller by a factor of about $1.5\,\Delta T/T_0$: under 2% for a rise of 20 K from 293 K.

Near room temperature, radiation is also orders of magnitude smaller than conduction, because tungsten's emissivity is low (a few percent) and $4\sigma T_0^3$ is only about 6 W/m²K. Either way, the total loss is $G\,\Delta T$ for some thermal conductance $G$.

(What matters is that the loss is linear in $\Delta T$, which holds when the temperature rise is small. It doesn't require radiation itself to be negligible.)

The three relations are:

- power in: $P = I^2 R$;
- power out: $P = G\,\Delta T$;
- resistance: $R = R_0(1 + \alpha\,\Delta T)$.

Eliminating $\Delta T$:

$$R = R_0\left(1 + \frac{\alpha I^2 R}{G}\right) \quad\Rightarrow\quad R = \frac{R_0}{1 - c\,I^2}, \qquad c = \frac{\alpha R_0}{G}$$

When the self-heating is small ($cI^2 \ll 1$), this is $R \approx R_0(1 + cI^2)$: a straight line in $I^2$ with intercept $R_0$ and slope $R_0 c$. The fit in 3.4 is that line.

**The linearisation's error is negligible here.** The neglected term is of order $(cI^2)^2$. For the default setpoints, fitting a straight line to the exact curve biases the intercept by:

| Resistance rise at 300 mA | Intercept bias |
| ------------------------- | -------------- |
| 2%                        | −0.008%        |
| 5%                        | −0.05%         |
| 10%                       | −0.19%         |

At about 10 mW into a 0.1 Ω filament the rise is expected to be a few percent at most. A larger rise would show as curvature and a large $\chi^2_\nu$ (3.4.2). The equations above are exactly linear in other variables too, $1/R$ against $I^2$ or $R$ against $P = VI$, but at these heating levels the difference is negligible.

**The filament's uneven temperature doesn't break this.** Its legs run cooler than its centre, but with linear heat transport the temperature profile keeps the same shape and simply scales with the power. So $R - R_0$ is still proportional to the power to first order, with local values of $\alpha$ and $G$ folding into an effective $c$. That's why the non-uniform temperature profile, which rules out inferring temperature from resistance, doesn't affect this extrapolation.

# Status of assumptions

Confirmed:

- The DP932E has the DP900-HIRES option: 1 mA programming and 0.1 mA readback resolution, which is what `CURRENT_READBACK_RESOLUTION_AMPS` assumes. A side effect of enabling it is that `*IDN?` reports `DP932A`. The supply is a DP932E, and the DP932E's specifications are the ones that apply.
- Each `:MEASure:CURRent?` takes a new measurement, and measurements that follow each other too closely hold up setpoint changes. See "Appendix: the supply's current readback".
- A 1× probe is on the scope's channel 1. The PVP3150 has a 1×/10× switch, so the procedure still asks the operator to check it (step 1.4). A probe switched to 10× would read 10× low while the scope is set to 1×, and would add an attenuation tolerance that isn't in the budget in 3.4.
- Both instruments are within their calibration interval (scope 18 months, supply 12 months), so the data-sheet bounds used in 3.4 apply.

All SCPI commands in this document have been checked against the DP900 and DHO800/DHO900 programming guides.

The defaults are tuned for filaments with $R_0 \approx 0.1\ \Omega$ (10–30 mV over 100–300 mA). "Other filaments" below says what to change for filaments far from that.

# Implementation order

The changes split into ten commits. Each one must build, pass `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check`, and run end to end with `--mock` (ARCHITECTURE.md §19).

If clippy flags something as unused before the commit that uses it, mark it `#[expect(dead_code)]` with a comment naming that commit, and remove the attribute there. `expect` rather than `allow`, so the attribute fails the build if it's forgotten.

Commits 5 and 6 don't depend on 2–4, and can be done before them.

1. **Make `Polarity` serialisable.**
   - Hoist `serde` to `[workspace.dependencies]`, add it to `host`, and derive `Serialize`/`Deserialize` on `Polarity`.
   - Update A7 and §7.2 in ARCHITECTURE.md.
   - No behaviour change.
2. **Add regulation mode and overcurrent protection to the supply.**
   - `RegulationMode` (serialisable), `get_regulation_mode`, `set_overcurrent_protection` and `get_overcurrent_tripped` in `host`, plus the `OFFMode` and OCP-clear additions to `reset`.
   - The pause between current measurements in `get_current`.
   - The matching `FilamentSystem` methods, adapters and mocks.
   - Check on the rig: with the output on into the filament, the mode reads `CC`.
3. **Read the scope from a single acquisition.**
   - The explicit `reset` settings, and `get_voltages` replacing `get_voltage` and `ClippedVoltage`.
   - `FilamentSystem::get_filament_voltages`, `FilamentSnapshot::filament_voltage: Option<f64>`, and "Clipped" in the filament block.
   - The signed mock voltage.
   - `ProcedureError::Check`, first used here: the stub `measure_at` records the average and fails with `Check` if the reading is clipped.
   - Check on the rig: the stub still measures. Temporarily setting a scale far too small shows "Clipped" in the filament block without ending the run.
4. **Add `set_vertical_scale` to `FilamentSystem`**, with its adapter (over the existing `host` method) and mock.
5. **Switch the Python scripts to JSON on stdin**, with shared schemas (see "`python/`" below).
   - `schemars` as a workspace dependency, derived alongside `serde` wherever a type crosses the boundary (including `Polarity` in `host` and `Measurement`).
   - `jsonschema` in `requirements.txt`.
   - `run_script` writes a serialisable input to stdin.
   - The input and output types for `mean_and_standard_error.py`, the schema snapshot test, the generated `python/schemas/`, and `python/script_io.py`.
   - `mean_and_standard_error.py` reads `{"samples": [...]}`.
   - Update ARCHITECTURE.md §4 (layout) and §12.
   - Check: the stub still records measurements, under `--mock` and on the rig.
6. **Add the analysis scripts.**
   - `numpy` and `scipy` in `requirements.txt`, and the extended `check_environment`.
   - The input types (in `python.rs`), and the output and parameter types (`Derived`, `ColdResistancePointAnalysis`, `ColdResistanceAnalysis` and `ColdResistanceFitParameters` in `results.rs`), for both scripts, with their wrappers and regenerated schemas.
   - `cold_resistance_point.py`, `cold_resistance_fit.py` and `test_cold_resistance.py`.
   - A `python` job in `.github/workflows/ci.yml`, added here rather than earlier because `python -m unittest` fails when there are no tests. It sets up Python 3.14 (the venv's version) with `actions/setup-python@v7`, runs `pip install -r requirements.txt`, and runs `python -m unittest`, all in `filament-characterisation/characteriser/python`.
   - Done when `python -m unittest` and the schema test pass.
7. **Lay the procedure's groundwork.**
   - The `constants.rs` changes.
   - `prepare`: the probe and warm-up confirmations, and the removal of the 30 V limit.
   - `pump_down`'s wait for `FILAMENT_OPERATING_PRESSURE_MBAR`.
   - The helpers `check_conditions`, `ramp_to`, `change_polarity`, `take_samples` and `wait_for_settle`, and the procedure constants.
   - Rewrite the stub `measure_cold_resistance` to use the helpers at a single current (still recording into the old `cold_*` fields), so they're exercised on the rig before the real procedure exists. Set the cold-resistance voltage limit and OCP at its start.
   - Include the Rust tests for `ramp_to` and `change_polarity`.
8. **Measure the setpoints.**
   - In `results.rs`: `ColdResistance` (with `analysis` still `None`) and `ColdResistancePoint`, replacing the four `cold_*` fields.
   - The room- and chamber-temperature prompts in 3.0, the effective bounds, and the end chamber-temperature prompt in 3.2.
   - Sections 3.0–3.2 and the per-point analysis (3.3).
   - Check on the rig: a full run records nine points, each with a resistance.
9. **Fit and report.**
   - 3.4 (including the correction to 20 °C, 3.4.7) and 3.5.
   - The `--mock` Rust test for $R_0 = 0.096$ Ω.
10. **Tidy up.**
    - Remove anything the stub left behind that's now unused.
    - Make sure every `expect(dead_code)` is gone.
    - Check the acceptance criteria.

# Changes outside the procedure

## `constants.rs`

- Raise `MAXIMUM_HEATING_CURRENT_AMPS` to 0.3, the largest cold-resistance setpoint. `set_heating_current_limit` refuses anything above it, and 300 mA dissipates only about 10 mW in a 0.1 Ω filament.
- Fix the doc comment on `MAXIMUM_HEATING_VOLTAGE_VOLTS`: the DP932E's channels 1 and 2 go to 30 V, not 32 V (32 V is the DP932A/U). The constant stays as a general ceiling. The cold-resistance measurement sets its own, much lower, limit (see below).
- Add `FILAMENT_OPERATING_PRESSURE_MBAR = 1e-5`. This is the pressure the chamber must reach before any current flows, and it resolves the `TODO` in `pump_down`. It's an arbitrary but comfortable threshold.
- Add `FILAMENT_ABORT_PRESSURE_MBAR = 5e-5`. If the pressure rises above this while the filament is powered, the procedure stops.

## `host::power_supply::PowerSupply`

The commands below are confirmed against the DP900 programming guide; section numbers refer to it. The existing methods' commands are also confirmed as they are: `:MEASure:CURRent? CH1` (4.6.2, returning e.g. `0.0500`, four decimals to match the 0.1 mA HIRES resolution), `:OUTPut CH1,<0|1>` and `:OUTPut? CH1` (4.8.16), and `:SOURce1:CURRent` / `:SOURce1:VOLTage` (4.9.1, 4.9.7).

Add these methods, following the existing style (lock the state, one USBTMC transaction, parse):

- **`get_regulation_mode() -> Result<RegulationMode, _>`**, with a new `RegulationMode { ConstantCurrent, ConstantVoltage, Unregulated }` enum beside `Polarity`. Use `:OUTPut:MODE? CH1` (4.8.2), which returns `CV`, `CC` or `UR`. `:OUTPut:CVCC? CH1` (4.8.1) is documented identically.
- **`set_overcurrent_protection(current: f64) -> Result<(), _>`**. It sends three commands:
  1. `:OUTPut:OCP:VALue CH1,<amps>` (4.8.8) sets the level.
  2. `:OUTPut:OCP:DELay CH1,10` (4.8.6) sets the delay. This is how long OCP is ignored after the output changes, so it covers a turn-on overshoot when `change_polarity` re-enables the output at a setpoint. 10 ms is the default, but set it explicitly rather than inheriting whatever it was left at.
  3. `:OUTPut:OCP CH1,ON` (4.8.7) enables it.

  When OCP trips, the supply disables the output itself.

- **`get_overcurrent_tripped() -> Result<bool, _>`**: `:OUTPut:OCP:QUES? CH1` (4.8.4). It returns `1` or `0` (an example in the guide shows `YES`/`NO`, but testing confirmed `1`/`0`), so error on anything else.

Extend `reset` to put two more things into a known state before anything else. Both go first, before the output is disabled:

1. **`:OUTPut:OFFMode 0V` (4.8.9).** This is a global setting, not per channel. It makes a disabled output actively hold 0 V, so by the time `set_polarity` switches the relays the output really is at 0 V. The alternatives don't guarantee that. `DELAYOFF` turns the output off after a delay, so the relays could switch while it was still live. `IMMEOFF` doesn't guarantee its fall time. 0V is the default, but it persists, so set it explicitly.
2. **`:OUTPut:OCP:CLEar CH1` (4.8.5)**, which clears any OCP event left latched from a previous run.

Make **`get_current`** pause between measurements. Each `:MEASure:CURRent?` takes a new measurement of about 180 ms, and measurements that follow each other too closely stop the supply applying setpoint changes (see "Appendix: the supply's current readback").

- Keep the `Instant` the last measurement's reply arrived in the supply's state.
- Before each query, while holding the lock, sleep until `MEASUREMENT_PAUSE` has passed since then. `MEASUREMENT_PAUSE` is 200 ms, a constant in `power_supply.rs`.
- Enforcing the pause in `host` means it covers every caller: the procedure's samples and the snapshot poll, which would otherwise interleave their measurements.
- Setpoint writes are unaffected, apart from waiting for the lock. In the tests, with this pause, a setpoint written just before a measurement always showed in the measurement after it, about 0.6 s later.

Add `serde` (with `derive`) to `host`, hoisted to `[workspace.dependencies]` as in ARCHITECTURE.md §4.1, and derive `Serialize` and `Deserialize` on `Polarity` and `RegulationMode` so `results.rs` can store them directly. This supersedes assumption A7; update A7 and §7.2 in ARCHITECTURE.md to say so.

## `host::oscilloscope::Oscilloscope`

The commands below are confirmed against the DHO800/DHO900 programming guide; section numbers refer to it. The existing methods' commands are also right as they are: `:STOP`, `:RUN`, `:CLEar`, `:MEASure:CLEar`, `:CHANnel<n>:DISPlay`, `:CHANnel1:OFFSet`, `:CHANnel1:SCALe` and `:TIMebase:SCALe`. `:CHANnel1:SCALe?` returns scientific notation (e.g. `1.000000E-01`), so `set_vertical_scale`'s read-back comparison parses correctly.

- **Make `reset` explicit about everything the measurement depends on**, since none of it may be left to however the scope was last used:
  - DC coupling (`:CHANnel1:COUPling DC`, 3.6.2).
  - Probe ratio 1× (`:CHANnel1:PROBe 1`, 3.6.8). Set it before the vertical scale, because the probe ratio changes the scale's valid range.
  - 20 MHz bandwidth limit (`:CHANnel1:BWLimit 20M`, 3.6.1). This is the only limit the DHO800 offers; the alternative is `OFF`, i.e. full bandwidth. The signal is DC, so everything above it is noise, and the high-frequency part inflates VMAX and VMIN, which would push the scale search to a coarser scale than the signal needs.
  - Normal acquisition mode (`:ACQuire:TYPE NORMal`, 3.3.3). Not `AVERages` (which averages whole waveforms across acquisitions, so consecutive samples wouldn't be independent) or `PEAK` (which exaggerates noise).
  - Auto trigger sweep (`:TRIGger:SWEep AUTO`, 3.27.4). This makes the scope acquire even when nothing triggers it, so the display shows a live trace before the first measurement. A DC signal never crosses the trigger level, so in normal sweep the screen would sit waiting. The first `get_voltages` call switches the sweep to single, because `:SINGle` is equivalent to `:TRIGger:SWEep SINGle` (3.1.4), and it stays there.
  - Measurements over the main time base (`:MEASure:AREA MAIN`, 3.17.19), not a zoom window.
  - VAVG, VMAX and VMIN measurement items on channel 1, added with the command form (`:MEASure:ITEM VAVG,CHANnel1` etc., 3.17.2) as `get_voltage` already does for VAVG.

  The existing 2 ms/div timebase stays: 10 div × 2 ms = 20 ms is exactly one 50 Hz mains cycle, so each VAVG averages out mains pickup and the supply's 100 Hz rectifier ripple.

- **Replace `get_voltage` with `get_voltages() -> Result<Voltages, OscilloscopeError>`**, where `Voltages { average, maximum, minimum }` holds each value as an `Option<f64>`. `None` means the scope still reported its invalid value (9.9E37) after the existing retries. Remove `OscilloscopeError::ClippedVoltage`. Because `reset` now adds all three measurement items, `get_voltages` doesn't add any: remove the lazy `:MEASure:ITEM VAVG,CHANnel1` from the old `get_voltage`, and the `is_measuring_voltage` flag in the oscilloscope's state that tracked it. The average alone can't detect clipping: it stays defined while the peaks are clipped, and is then biased, because the clipped samples are pinned at the edge of the range. While holding the lock:
  1. Arm a single acquisition (`:SINGle`, 3.1.4).
  2. Poll `:TRIGger:STATus?` (3.27.3), which returns `TD`, `WAIT`, `RUN`, `AUTO` or `STOP`, sleeping 50 ms between queries:
     - On `WAIT`, the scope is armed and waiting. Send `:TFORce` (3.1.5) once, since a DC signal may never cross the trigger level. The guide says `:TFORce` works in single mode.
     - On `STOP`, the acquisition is complete. Noise crossing the trigger level can trigger it before the force is sent; that's fine, because it's still an acquisition that began after the arm.
     - Anything else, keep polling.
     - If `STOP` doesn't arrive within `ACQUISITION_TIMEOUT` (2 s, a constant in `oscilloscope.rs`; an acquisition has been seen to take ~360 ms), return an error. Waiting for `WAIT` before forcing avoids sending the force before the scope has filled its pre-trigger buffer and armed, when it might be ignored and leave the scope waiting forever.
  3. Query `:MEASure:ITEM? VAVG,CHANnel1`, then `VMAX` and `VMIN`, each with the existing retry-on-9.9E37 logic. The scope is stopped, so all three come from the same acquisition. The guide doesn't give the invalid value itself; it only says that results out of the valid range are invalid. 9.9E37 is what your existing code has observed, and that's what `CLIPPED_VOLTAGE` encodes.

  Taking a single acquisition, rather than querying a running scope at least 20 ms apart, guarantees three things:
  - **All three values come from the same acquisition.** On a running scope, three separate queries can straddle two acquisitions.
  - **The acquisition started after the call,** so it reflects the state after the last change of current, polarity or scale. A running scope can return a value from an acquisition that began before the change.
  - **Samples are independent however fast the scope's measurement engine updates.** A running scope's cycle is the 20 ms window plus dead time and measurement processing, and that total isn't specified, so a 20 ms spacing doesn't guarantee a new value.

  The snapshot poll uses the same method, which is fine: the lock makes the whole sequence atomic. The scope is left stopped between samples, which only means the display updates once per sample.

## `hardware` module

- **Stop treating a clipped voltage as a hardware failure.** At the moment `RealFilamentSystem::snapshot` propagates `OscilloscopeError::ClippedVoltage`. Three consecutive clipped polls would then end the run as `HardwareFailed` (§8.1), and the vertical-scale search in 3.1 deliberately produces clipped readings. Change `FilamentSnapshot::filament_voltage` to `Option<f64>`, matching `Voltages`. It's the average, or `None` if _any_ of the average, maximum and minimum is `None`, because an average with clipped peaks is biased and shouldn't be displayed as if it were good. Say so in its doc comment, since that's a slightly wider meaning of `None` than on `Voltages`. The headroom check belongs to the procedure, not the snapshot. Render `None` as "Clipped" in the filament block. A clipped reading is still a real reading (§7.1), just not a number.
- Replace `get_filament_voltage` with `get_filament_voltages() -> Result<Voltages, HardwareError>`, returning `host`'s `Voltages` directly. Like `Polarity`, it's plain data rather than an instrument type, so it's an exception to the §7.5 rule that nothing outside `real.rs` names a `host` type; note that in §7.5.
- Extend `FilamentSystem` with:
  - `get_regulation_mode() -> Result<RegulationMode, HardwareError>`;
  - `get_overcurrent_tripped() -> Result<bool, HardwareError>`;
  - `set_overcurrent_protection(current: f64) -> Result<(), HardwareError>`;
  - `set_vertical_scale(scale: f64) -> Result<(), HardwareError>`, in volts per division;
- **Mocks** (§7.6). For `--mock` to get through the procedure's checks, the mock's filament voltage must be `MOCK_FILAMENT_RESISTANCE_OHMS` (0.096) × the set current × +1 for `Forward` / −1 for `Reverse`, or 0 V with the output off or `Nil`. The average, maximum and minimum are all that value. The mock current readback returns the set current, the regulation mode is always `ConstantCurrent`, and OCP never trips. This is still a contract rather than a simulation: no noise, no time dependence, no heating.

## `procedure` module

Add `ProcedureError::Check(String)` for the procedure's own checks, as §11.1 anticipates: clipping after the scale is chosen, leaving constant-current mode, an OCP trip, a pressure excursion, or a failed polarity sanity check. Log with `error!` before returning it, as usual.

## `results.rs`

Replace the four `cold_*` fields on `Characterisation` with `cold_resistance: Option<ColdResistance>`. Polarities are stored as `host`'s `Polarity`, now serialisable; they're never `Nil`.

```rust
/// A value derived by Python from measurements, with its standard uncertainty.
///
/// Unlike `Measurement` there are no samples: it's calculated from other
/// quantities, not sampled.
pub struct Derived {
    pub uncertainty: f64,
    pub value: f64,
}

/// Everything recorded while measuring the cold resistance.
pub struct ColdResistance {
    /// The operator-entered chamber temperature after the setpoints (3.2),
    /// once they've all been measured.
    pub chamber_end_temperature_celsius: Option<f64>,

    /// The operator-entered chamber temperature before the setpoints (3.0).
    pub chamber_start_temperature_celsius: f64,

    /// The fit and uncertainty budget, once every setpoint has been measured.
    pub analysis: Option<ColdResistanceAnalysis>,

    /// Every scalar input to the fit (3.4), recorded when the fit runs.
    pub fit_parameters: Option<ColdResistanceFitParameters>,

    /// The operator-entered room temperature near the supply (3.0).
    pub room_temperature_celsius: f64,

    /// One entry per setpoint, in the order measured.
    pub points: Vec<ColdResistancePoint>,

    /// The polarity that gave a positive voltage (3.1.4), once the scale
    /// search has finished.
    pub positive_polarity: Option<Polarity>,

    /// The vertical scale used for every measurement, once the scale search
    /// has finished (3.1).
    pub vertical_scale_volts_per_division: Option<f64>,
}

/// The fit's scalar inputs. With `points`, this is everything
/// `cold_resistance_fit.py` needs, so the fit can be re-run from the results
/// file alone, and anyone reading the file can see which assumptions produced
/// the result.
pub struct ColdResistanceFitParameters {
    /// The effective supply readback bounds after temperature correction (3.0).
    pub current_gain_bound: f64,
    pub current_offset_bound_amps: f64,

    /// $T_f$ and its bound $a_T$ (3.4.7).
    pub filament_temperature_bound_kelvin: f64,
    pub filament_temperature_celsius: f64,

    pub reference_temperature_celsius: f64,

    /// $\alpha$ and its bound $a_\alpha$ (3.4.7).
    pub temperature_coefficient_bound_per_kelvin: f64,
    pub temperature_coefficient_per_kelvin: f64,

    /// The bound on the oscilloscope's gain error (3.4.3).
    pub voltage_gain_bound: f64,
}

/// The measurements at one setpoint.
pub struct ColdResistancePoint {
    /// The output of `cold_resistance_point.py` (3.3), once it's run.
    pub analysis: Option<ColdResistancePointAnalysis>,

    /// The relay state measured first. With `positive_polarity`, this says
    /// which measurement came first and so which settle followed the ramp from
    /// the previous setpoint. It's derivable from the alternation rule
    /// (3.2.2.1), but recorded so the file doesn't depend on that rule.
    pub first_polarity: Polarity,

    /// The measurements in the polarity that gives a negative voltage.
    pub negative_current_amps: Measurement,
    pub negative_settle_seconds: f64,
    pub negative_voltage_volts: Measurement,

    /// The measurements in the polarity that gives a positive voltage.
    pub positive_current_amps: Measurement,
    pub positive_settle_seconds: f64,
    pub positive_voltage_volts: Measurement,

    pub setpoint_amps: f64,

    /// Warnings raised while measuring this point, e.g. a settle timeout.
    pub warnings: Vec<String>,
}

/// The per-point analysis: exactly the output of `cold_resistance_point.py`.
pub struct ColdResistancePointAnalysis {
    pub current_amps: Derived,
    pub current_squared_amps_squared: Derived,
    pub offset_voltage_volts: Derived,
    pub resistance_ohms: Derived,
    pub voltage_volts: Derived,

    /// Warnings raised by the analysis, e.g. a large offset voltage.
    pub warnings: Vec<String>,
}

/// The fit and uncertainty budget: exactly the output of `cold_resistance_fit.py` (3.4).
pub struct ColdResistanceAnalysis {
    pub chi_squared_p_value: f64,
    pub current_gain_uncertainty_ohms: f64,
    pub current_offset_uncertainty_ohms: f64,

    /// Statistical uncertainty after Birge scaling (3.4.2).
    pub fit_uncertainty_ohms: f64,

    /// $R_0$ at $\delta = -\delta_{max}$ and $+\delta_{max}$ (3.4.5).
    pub offset_corner_resistances_ohms: [f64; 2],

    pub reduced_chi_squared: f64,

    /// $R_{20}$, the cold resistance corrected to 20 °C (3.4.7), with $u_c(R_{20})$.
    pub reference_resistance_ohms: Derived,

    /// The nominal $R_0$, at the filament temperature, with $u_c(R_0)$ (3.4.6).
    pub resistance_ohms: Derived,

    pub voltage_gain_uncertainty_ohms: f64,

    /// The correction's own uncertainty terms (3.4.7).
    pub temperature_coefficient_uncertainty_ohms: f64,
    pub temperature_uncertainty_ohms: f64,

    pub slope_ohms_per_amp_squared: Derived,

    /// $G = \alpha R_0^2/b$ (3.5), or `None` if $b$ isn't clearly positive
    /// (more than twice its uncertainty), when dividing by it is meaningless.
    pub thermal_conductance_watts_per_kelvin: Option<Derived>,

    pub warnings: Vec<String>,
}
```

(Derives, including `JsonSchema`, and doc comments per §18, with fields in alphabetical order; the comments above are abbreviated.) Expanded uncertainties aren't stored: they're $2u_c$ and are computed for display (3.5). `current_offset_uncertainty_ohms` is the offset corner term; `fit_uncertainty_ohms` is the Birge-scaled statistical term.

Create the `ColdResistance` at the end of 3.0, once both temperatures are known, and save it straight away. Record and `ctx.save()` again after the scale is chosen, after every setpoint, after the end temperature, and after the analysis. The four `Option` fields are exactly the ones not yet known at those save points, so an aborted run still leaves everything measured up to that point on disk.

## `python/`

Every script, including the existing `mean_and_standard_error.py`, reads one JSON object on stdin and writes one JSON object on stdout. Errors go to stderr with a non-zero exit, as now. This replaces §12's "arguments in"; update §12 and the §4 layout to match.

**The Rust side.** `run_script<I: Serialize, O: DeserializeOwned>(script, &input)`:

1. Spawn the interpreter with stdin piped.
2. Write `serde_json::to_vec(&input)` to stdin.
3. **Drop the stdin handle to close the pipe before waiting.** Otherwise the script's `json.load(sys.stdin)` never sees end-of-file and blocks until the timeout.
4. `wait_with_output` inside the existing timeout.

**Shared schemas.** The Rust types are the source of truth.

- **Deriving.** Every input and output type derives `Serialize`, `Deserialize` and `schemars::JsonSchema`, with `#[serde(deny_unknown_fields)]`.
  - Input types live in `python.rs`.
  - Output types that are stored live in `results.rs`, and each script's output is exactly one of them, so there's no mapping step: `ColdResistancePointAnalysis` for the point script, `ColdResistanceAnalysis` for the fit.
  - `Polarity` and `Measurement` derive `JsonSchema` too.
- **The snapshot test.** A Rust test, `schemas_are_up_to_date`, generates each script's input and output schema with `schemars::schema_for!`. It compares them with the committed files `python/schemas/<script>.input.json` and `python/schemas/<script>.output.json`, and fails if they differ. Run with `UPDATE_SCHEMAS=1`, it writes them instead. A change to a Rust type therefore can't silently diverge from what Python expects.
- **The Python helper.** `python/script_io.py` provides two functions that every script uses:
  - `read_input(script_name)` loads the input schema, reads stdin, validates it with `jsonschema.validate`, and exits with the validation error on stderr if it doesn't match.
  - `write_output(obj)` writes the output with `json.dump(obj, sys.stdout, allow_nan=False)`. By default Python writes `NaN` and `Infinity`, which aren't valid JSON and which `serde_json` rejects, so `allow_nan=False` makes that fail in Python with a clear message instead.
- **Output checks.** The Python tests validate each script's output against its output schema, and `serde` validates it again when Rust deserialises it.

**The inputs:**

- `mean_and_standard_error.py`: `{"samples": [f64, ...]}`. Its output is unchanged: `{"value", "uncertainty"}`.
- `cold_resistance_point.py` (3.3):
  - `positive_voltage_volts`, `negative_voltage_volts`, `positive_current_amps` and `negative_current_amps`, each a `Measurement` (samples, value, uncertainty);
  - `current_resolution_amps`;
  - `offset_warning_volts`.
- `cold_resistance_fit.py` (3.4):
  - `points`, each with `voltage_volts` and `current_amps` as `Derived` values (from each point's `ColdResistancePointAnalysis`);
  - `parameters`, a `ColdResistanceFitParameters`.

**Other changes:**

- `requirements.txt`: pin `numpy`, `scipy` and `jsonschema` alongside `uncertainties`.
- `python.rs::check_environment`: import all four (`import uncertainties, numpy, scipy, jsonschema`) and update its error message.
- `test_cold_resistance.py`, runnable with `python -m unittest`, covering the cases in "Tests" below.

## `Cargo.toml`

Add `rand` for the shuffle, and `schemars` (as a workspace dependency, also used by `host`). No seed needs recording: the order actually used is in `points`, and each point's `first_polarity`.

# Constants for the procedure

These go at the top of `characterisation.rs`, one doc comment each, replacing the stub `COLD_RESISTANCE_CURRENT_AMPS` and `SETTLING_TIME`. `SAMPLE_COUNT` stays at 20.

| Constant                                                                                                       | Value                                        | Why                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| -------------------------------------------------------------------------------------------------------------- | -------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `COLD_RESISTANCE_SETPOINTS_AMPS`                                                                               | 0.100, 0.125, …, 0.300 (9 points)            |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `RAMP_STEP_AMPS` / `RAMP_STEP_INTERVAL`                                                                        | 0.005 A / 100 ms                             | 50 mA/s. At these currents ramping has no physical benefit; it's kept so `ramp_to` is ready for hot measurements, where each jump should be small compared with what the filament's thermal time constant smooths out. Ten commands a second is trivial for the supply.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `COLD_RESISTANCE_VOLTAGE_LIMIT_VOLTS`                                                                          | 2.0                                          | ~0.15 V is needed. A low limit keeps the channel in constant current while limiting the energy delivered if the circuit opens and re-closes, which 30 V wouldn't.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `OVERCURRENT_PROTECTION_MARGIN_AMPS`                                                                           | 0.05                                         | OCP = largest setpoint + margin, as a hardware backstop. The OCP delay (10 ms) should ride out any turn-on overshoot when the output is re-enabled at a setpoint. If OCP trips anyway, widen the margin or lengthen the delay.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `VERTICAL_SCALES_VOLTS_PER_DIVISION`                                                                           | 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1, 2, 5, 10 | Starts at 10 mV/div so the ±1% gain spec is guaranteed; it's only "typical" at ≤5 mV/div                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `CLIPPING_HEADROOM`                                                                                            | 0.8                                          | A reading counts as clipped if any of VAVG, VMAX and VMIN is invalid, or if $\max(\lvert V_{max}\rvert, \lvert V_{min}\rvert) > 0.8 \times 4\ \text{div} \times \text{scale}$. The scale is chosen from a few readings taken right after the ramp, then fixed. Later readings are bigger: the filament's resistance keeps rising as it settles at the top setpoint, a later 20 ms window can catch a larger noise peak, and offsets and thermal EMFs drift. Without margin, a scale that only just fits would clip mid-run and abort it. The margin also makes clipping deterministic, rather than dependent on exactly where the scope's firmware starts reporting 9.9E37. It costs almost nothing, because the gain uncertainty doesn't depend on the scale (3.4.3) and 12-bit resolution averaged over a 20 ms window is far finer than needed. |
| `RELAY_SETTLE_TIME`                                                                                            | 50 ms                                        | Output off while relays move                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `SETPOINT_APPLY_TIME`                                                                                          | 1 s                                          | Wait before a single reading is taken as being at a new setpoint; the supply can take about 0.6 s to apply one (see the readback appendix)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `SETTLE_WINDOW` / `SETTLE_TOLERANCE` / `MAXIMUM_SETTLE_TIME`                                                   | 2 s / 2e-4 / 60 s                            | See `wait_for_settle`. The earliest possible settle is 2 × `SETTLE_WINDOW` = 4 s.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `CURRENT_READBACK_RESOLUTION_AMPS`                                                                             | 1e-4                                         | DP932E with HIRES                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `CURRENT_GAIN_BOUND`                                                                                           | 0.0015                                       | DP900 readback accuracy, % part                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `CURRENT_OFFSET_BOUND_AMPS`                                                                                    | 0.005                                        | DP900 readback accuracy, offset part                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `CURRENT_GAIN_TEMPERATURE_COEFFICIENT_PER_CELSIUS` / `CURRENT_OFFSET_TEMPERATURE_COEFFICIENT_AMPS_PER_CELSIUS` | 1e-4 / 0.002                                 | Applied outside 20–30 °C                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `REFERENCE_TEMPERATURE_CELSIUS`                                                                                | 20                                           | The temperature $R_0$ is corrected to (3.4.7)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `TUNGSTEN_TEMPERATURE_COEFFICIENT_PER_KELVIN`                                                                  | 0.0045                                       | $\alpha$ for tungsten near room temperature                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `TUNGSTEN_TEMPERATURE_COEFFICIENT_BOUND_PER_KELVIN`                                                            | 0.0003                                       | Half-width on $\alpha$. This is a judgement: published values span roughly 0.0042–0.0048 /K depending on purity and doping.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `THERMOMETER_BOUND_KELVIN`                                                                                     | 1.0                                          | Half-width covering the thermometer's accuracy plus how far the filament might sit from the flange temperature. This is a judgement; tighten it if the thermometer and placement justify it.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `VOLTAGE_GAIN_BOUND`                                                                                           | 0.02                                         | 3.4.3; 0.01 under the span reading                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |

# The procedure

## 1. Preparing (changes to `prepare`)

1. Keep the existing filament ID prompt first.
2. Remove the `set_heating_voltage_limit(MAXIMUM_HEATING_VOLTAGE_VOLTS)` call. Each measurement section now sets the limit it needs; the cold-resistance section sets its own in 3.0.
3. Keep the existing "filament is mounted" and "chamber is sealed" confirmations, and add two more:
   - "Confirm that a 1× probe is connected to the oscilloscope's channel 1 and that its switch is set to 1×." `reset` has set the scope's probe ratio to 1×, so this is what makes that correct.
   - "Confirm that the oscilloscope has been on for at least 30 minutes and the power supply for at least 60 minutes." The supply's accuracy figures assume a 1-hour warm-up, and the pump-down usually covers it.

## 2. Pumping down (changes to `pump_down`)

After the TMP reaches speed, `ctx.wait_for` the pressure to fall below `FILAMENT_OPERATING_PRESSURE_MBAR`, resolving the existing `TODO`.

## 3. Measuring cold resistance (`measure_cold_resistance`)

### Helpers

Private async functions in `characterisation.rs`.

- **`check_conditions(ctx, hardware)`:** fails with `ProcedureError::Check` if `ctx.snapshot()` shows the pressure above `FILAMENT_ABORT_PRESSURE_MBAR` or the TMP not running. Call it between every sample and every ramp step.
- **`ramp_to(target)`:** step the current towards `target` by `RAMP_STEP_AMPS` every `RAMP_STEP_INTERVAL`, finishing exactly on `target`.
- **`change_polarity(polarity)`:** disable the output, `set_polarity`, sleep `RELAY_SETTLE_TIME`, and enable the output again at the unchanged setpoint. `set_polarity` already refuses while the output is on. There's deliberately no ramp down and back up. The filament dissipates the same power in either polarity, and at ≤300 mA the jump is harmless. Ramping would only lengthen the time the filament spends cooling, and so the settle that follows.
- **`take_samples(n)`:** `n` pairs of (`get_filament_voltages`, `get_heating_current`), then `get_overcurrent_tripped` and `get_regulation_mode`, after every sample. The average is what's recorded. Each current reading is a new measurement of about 180 ms, and `get_current` pauses 200 ms before each one, so a sample takes about 0.4 s and 20 samples about 8 s. No extra spacing is needed: every reading is already independent.
  - An OCP trip fails with `Check` ("overcurrent protection tripped"). Check this before the mode, because a tripped output also isn't in constant current, and the OCP message is the more useful one.
  - Anything other than `ConstantCurrent` fails with `Check`, since the filament circuit is probably open.
  - A clipped reading (as defined by `CLIPPING_HEADROOM`) fails with `Check` ("clipped; the scale must not change mid-run, restart the run").
  - Each `get_filament_voltages` is a fresh 20 ms acquisition (see the oscilloscope changes), so samples are independent.
- **`wait_for_settle()`:** inside `ctx.waiting`, sample repeatedly at the same cadence as `take_samples`, without recording the samples, and fail on clipping as `take_samples` does.
  - For each sample, compute $R_{quick} = V_{avg}/I$. It comes from one polarity, so it includes offsets and thermal EMFs, but those are constant over seconds and cancel in the comparison below.
  - After each new sample, once at least 2 × `SETTLE_WINDOW` has passed since the change, split the samples into the latest `SETTLE_WINDOW` (B) and the one before it (A). Compute each window's mean $m$ and standard error $\text{SE} = s/\sqrt n$ (sample standard deviation, `ddof = 1`), and let $u_{diff} = \sqrt{\text{SE}_A^2 + \text{SE}_B^2}$, which is ordinary propagation for a difference of two independent means.
  - Settled means $\lvert m_B - m_A\rvert < \max(\text{SETTLE\_TOLERANCE}\times m_B,\ 2\,u_{diff})$. The windows slide forward one sample at a time, so the earliest settle is 2 × `SETTLE_WINDOW`.
  - The two terms cover opposite situations, and the maximum means "the change is either negligible or undetectable".
    - When noise is low, $2u_{diff}$ is tiny and on its own would demand arbitrary flatness, so slow harmless drifts (offsets, the rig's temperature) could keep the run waiting until timeout. The tolerance sets a floor on what counts as flat enough.
    - When noise is high, a fixed tolerance could be exceeded by chance alone every time. $2u_{diff}$ accepts a difference that's indistinguishable from noise. Any drift that small is also small compared with the point's own uncertainty, which comes from the same noise, and whatever remains shows up in $\chi^2_\nu$ and the Birge inflation.
  - Choosing `SETTLE_TOLERANCE`: 2e-4 is about one point's statistical uncertainty in $R$ at the lowest setpoint. The current-quantisation floor alone is $0.1\ \text{mA}/\sqrt{12} \approx 0.029$ mA per polarity, which is about $2\times10^{-4}$ of 100 mA once the two polarities are averaged. So "settled" means any remaining drift is below what the point can resolve anyway. At that level the $2u_{diff}$ term will probably govern most of the time, which is fine. Treat 2e-4 as a starting point, and tune it from the logged settle times and per-point uncertainties after the first runs on the rig.
  - The supply can take about 0.6 s to apply a setpoint change (see `get_current`'s pause), so the first readings may still be at the previous current.
    - That's well inside the 4 s minimum, and any heating after the late change shows up as a difference between the windows.
    - Checking the readback against the setpoint wouldn't work instead: the readback offset bound (5 mA) is as large as the last ramp step.
  - This comparison is control flow, not a reported statistic, so it's fine in Rust (note that in a comment).
  - At `MAXIMUM_SETTLE_TIME`, add a warning to the point and carry on.
  - Return the settle time, which is recorded. These become useful when the thermal time constants are characterised later.

### 3.0 Setup

1. Disable the output, set the current to 0 A, and set the voltage limit to `COLD_RESISTANCE_VOLTAGE_LIMIT_VOLTS`.
2. Set OCP to the largest setpoint plus `OVERCURRENT_PROTECTION_MARGIN_AMPS`.
3. Ask for the chamber temperature: "Chamber temperature at the filament's flange". The unheated filament sits at roughly the temperature of its mount, which is in thermal contact with the flange, not with the room air. This is asked here rather than in `prepare` because the chamber can warm during the pump-down (the TMP runs warm).
4. Ask for the room temperature near the supply with `ctx.input::<f64>(…, Some("°C"))`. This is only for the supply's accuracy band; the filament's temperature is asked for in step 3 and at the end of 3.2.
   - The DP900 data sheet specifies readback accuracy at 25 °C ± 5 °C, and separately gives a temperature coefficient per °C (0.01% + 2 mA for current on CH1/CH2). It doesn't say how to combine them. The usual convention in instrument data sheets is to add the temperature coefficient for each degree outside the accuracy band.
   - So if the temperature is outside 20–30 °C, let $\Delta T_{out}$ be the number of degrees outside that band. The effective bounds are then $g_{max} = $ `CURRENT_GAIN_BOUND` $+ 10^{-4}\,\Delta T_{out}$ and $\delta_{max} = $ `CURRENT_OFFSET_BOUND_AMPS` $+ 0.002\,\Delta T_{out}$.
   - Show both as measurement steps.
5. Create the `ColdResistance` and save.

### 3.1 Choosing the vertical scale (sub-section)

Why this matters: the scope's gain error is only a single multiplicative factor on every voltage, and so on $R_0$, if every measurement is taken on the same scale, because each scale has its own gain error. The scale is chosen once here and never changed.

1. `set_vertical_scale` to the first entry of `VERTICAL_SCALES_VOLTS_PER_DIVISION`.
2. With the output disabled, `set_polarity(Forward)`, enable the output at 0 A, and `ramp_to` the largest setpoint. Wait `SETPOINT_APPLY_TIME` (1 s), since the supply can take about 0.6 s to apply a setpoint change, then take one reading with `get_filament_voltages`. While it's clipped (any value invalid, or VMAX/VMIN beyond the headroom), move to the next scale and re-read. Checking VMAX and VMIN rather than the average is what keeps the whole signal, noise included, on screen.
   - Nothing is read during the ramp. The largest setpoint gives the largest voltage, so a scale that fits it fits every setpoint, and clipping on the way up does no harm. Reading after every step would add an acquisition (about 0.36 s) to each of the 60 steps without changing the scale chosen.
3. `change_polarity(Reverse)`, wait `SETPOINT_APPLY_TIME` again, then take one reading and apply the same clipping rule (stepping the scale up while it clips).
4. Use the last unclipped average from step 2 as $V_F$ and the last one from step 3 as $V_R$; both were taken at the largest setpoint. The polarity whose voltage is positive is recorded as `positive_polarity`. Fail with `Check` ("the filament voltage doesn't reverse with the current; check the wiring") unless both of these hold:
   - **$V_F$ and $V_R$ have opposite signs.** If they don't, the relays aren't reversing the current through the filament, or the sense leads aren't across it.
   - **The non-reversing part is plausible:** $\lvert V_F + V_R\rvert/2 \le 2\times(0.1\ \text{div}\times\text{scale} + 2\ \text{mV})$, using the final scale.
     - Why these two quantities are related: $V_F = +V + V_{off}$ and $V_R = -V + V_{off}$, where $V_{off}$ is everything that doesn't reverse with the current. So $(V_F + V_R)/2 = V_{off}$ exactly. In a correctly wired rig, $V_{off}$ is the scope's DC offset error plus thermal EMFs, and the scope's offset error is bounded by its spec, $\pm(0.1\ \text{div} + 2\ \text{mV})$. Thermal EMFs in the sense circuit are only microvolts to tens of microvolts. So $\lvert V_{off}\rvert$ should be within the offset spec, and twice the spec leaves generous margin.
     - What exceeding it means: something other than the filament contributes a voltage that doesn't reverse. For example, a scope ground clip connected somewhere that changes with the relay state, a ground loop, or pickup. Those corrupt the measurement in ways reversal can't fix.
     - This is a plausibility check rather than a measurement. The sign check above is the essential one. If the limit ever proves too tight on a correctly wired rig, loosen it rather than removing it.

   This reuses readings already taken rather than taking new ones. If the scale went up during step 3, $V_F$ was read on the smaller scale, which doesn't matter for a sign check. The limits don't depend on $R_0$, so they suit any filament.

5. Ramp to 0 A. Show the scale and the positive polarity, record them, and save.

### 3.2 Measuring the setpoints

1. Shuffle `COLD_RESISTANCE_SETPOINTS_AMPS` at random. The random order stops slow drifts (e.g. thermal drift of the rig) from correlating with current.
2. For each setpoint $I_k$, in a sub-section titled "Measuring at $I_k$ mA ($k$/$N$)", where $k$ counts from 1 and $N$ is the number of setpoints, so the operator can see how far through they are:
   1. The first polarity is whichever the relays are already in. Since each setpoint ends in its second polarity, the order alternates (F→R, then R→F, and so on), and each new setpoint needs no extra flip. The alternation matters because a slowly drifting non-reversing voltage (the scope's offset, thermal EMFs as the rig warms) biases $V$ by $\pm d/2$, where $d$ is the drift between the two measurements, with the sign set by which polarity came first. A fixed order would make that a consistent bias at every point. Alternating makes it cancel on average, and the shuffled setpoint order keeps the alternation from correlating with current.
   2. `ramp_to(I_k)`.
   3. `wait_for_settle`, then `take_samples(SAMPLE_COUNT)`.
   4. `change_polarity(second)`, `wait_for_settle`, `take_samples(SAMPLE_COUNT)`.
   5. File each polarity's samples and settle time under `positive_*` or `negative_*` using `positive_polarity` from 3.1.4. That's the only place the relay-to-sign mapping is used. Everything downstream works with signs, so the analysis needs no convention about which relay state is which.
   6. Summarise the samples with `mean_and_standard_error.py`, as the stub's `summarise` does. That's four calls: voltage and current, for each of the two polarities.
   7. Call `cold_resistance_point.py` (3.3).
   8. Show $R$ as a measurement step (e.g. "Resistance: 96.12 ± 0.03 mΩ"), plus any warnings as text steps.
   9. Append the point to `points`, record, and save.
3. Ramp to 0 A and disable the output.
4. Ask for the chamber temperature at the flange again, and save.

### 3.3 Per-point analysis: `cold_resistance_point.py`

**Input and output:** see "`python/`". The output is a `ColdResistancePointAnalysis`, stored as the point's `analysis`. Only each `Measurement`'s value and uncertainty are used, not its samples.

Using `uncertainties`:

1. For each polarity, $u(\bar I_s) = \sqrt{\text{SE}_I^2 + \Delta_I^2/12}$, where $\text{SE}_I$ is the `Measurement`'s standard error and $\Delta_I$ is the readback resolution. The $\Delta_I^2/12$ term is a quantisation floor: the current is usually steadier than one resolution step, so every sample reads the same and the standard error is 0, but the value is still only known to within one step. The recorded `Measurement` keeps the plain standard error, as its doc comment promises. The voltage uses its standard error as is.
2. Fail with a clear error unless $\bar V_P > 0$ and $\bar V_N < 0$, since otherwise the relay-to-sign mapping from 3.1.4 didn't hold. Then, with $P$ and $N$ for the positive and negative polarities:
   1. $V = (\bar V_P - \bar V_N)/2$. Voltages that don't reverse with the current (scope DC offset, Seebeck/thermal EMFs, constant pickup) cancel.
   2. $I = (\bar I_P + \bar I_N)/2$. Both readbacks are positive, because the supply always sources positive current and the relays do the reversal.
   3. $R = V/I$.
   4. $x = I^2$.
3. As a diagnostic, $V_{off} = (\bar V_P + \bar V_N)/2$, the residual non-reversing voltage. Warn if $\lvert V_{off}\rvert$ exceeds `offset_warning_volts`, which Rust passes as the same limit as the check in 3.1.4.
4. Fail with a clear error if $V \le 0$ or $u(R) = 0$.

### 3.4 Fit and uncertainty budget: `cold_resistance_fit.py`

**Input and output:** see "`python/`". Each point's $u(I)$ is the floored value from 3.3. The output is a `ColdResistanceAnalysis`.

The fit takes $V$ and $I$, not just $R$, because the offset analysis in 3.4.5 recomputes $R$ from shifted currents.

#### 3.4.1 Weighted least squares

Fit $R = R_0 + b\,x$, weighting each point by $1/u(R_i)^2$, with `scipy.optimize.curve_fit`:

```python
def line(x, r0, b):
    return r0 + b * x

popt, pcov, info, _, _ = curve_fit(
    line, x, r, sigma=u_r, absolute_sigma=True,
    p0=[r[0], 0.0], full_output=True,
)
r0, b = popt
u_r0, u_b = np.sqrt(np.diag(pcov))
```

Using it correctly:

- **Pass `sigma=u_r`, the standard uncertainties, not weights.** `curve_fit` minimises $\sum_i \left((R_i - f(x_i))/\sigma_i\right)^2$, so it applies the $1/u^2$ weighting itself.
- **Pass `absolute_sigma=True`.** Without it, `curve_fit` treats `sigma` as relative and multiplies `pcov` by $\chi^2_\nu$. That silently applies the Birge scaling of 3.4.2 in both directions, including shrinking the uncertainty when $\chi^2_\nu < 1$. With it, `pcov` is the covariance implied by the stated uncertainties, and 3.4.2 applies the inflation deliberately.
- **Pass `p0`.** The default start of (1, 1) is far from the answer. The fit is linear, so it converges anyway, but a sensible start costs nothing.
- **Pass `full_output=True`,** so `info["fvec"]` provides the weighted residuals $(f(x_i) - R_i)/u(R_i)$ for 3.4.2.
- **Check the uncertainties before calling.** Every `u_r` must be finite and positive. A zero would make its weight infinite, and 3.3 already fails the point if $u(R) = 0$.
- **Keep the covariance.** `pcov[0, 1]` is the covariance between $R_0$ and $b$, which matters for anything computed from both, such as $G$ in 3.5. Use `uncertainties.correlated_values(popt, pcov)` for those.

The same call is reused for every refit in 3.4.5.

The uncertainty in $x$ is ignored in the fit. It enters as $b\,u(x_i)$, which is smaller than $u(R_i)$ by roughly the factor $2(R_i - R_0)/R_i$ (a few percent at these currents). Warn if $\max_i \lvert b\rvert\,u(x_i)/u(R_i)$ exceeds 0.1.

#### 3.4.2 Goodness of fit

$\chi^2 = \sum_i \left((R_i - R_0 - b\,x_i)/u(R_i)\right)^2$, which is `np.sum(info["fvec"] ** 2)` from 3.4.1. Its degrees of freedom are $\nu = N - 2$: nine points, less the two fitted parameters.

$\chi^2$ and the reduced $\chi^2_\nu = \chi^2/\nu$ carry the same information on different scales:

- **$\chi^2$** is expected to be about $\nu$ for a good fit, which is 7 here.
- **$\chi^2_\nu$** divides that out, so it's expected to be about 1 whatever the number of points. When people say "$\chi^2$ should be about 1", they mean $\chi^2_\nu$.

Both are reported. $\chi^2_\nu$ is the easy one to read, with a typical spread of $\pm\sqrt{2/\nu} \approx \pm0.5$ here. The p-value uses $\chi^2$ and $\nu$ directly.

(`scipy.stats.chisquare` is not for this. It computes Pearson's statistic for counts, $\sum (O-E)^2/E$, which assumes Poisson uncertainties.)

**The p-value.** If the straight-line model is right and the per-point uncertainties are right, $\chi^2$ follows a chi-squared distribution with $\nu$ degrees of freedom. The p-value, `scipy.stats.chi2.sf(chi2, nu)`, is the probability of getting a $\chi^2$ at least this large by chance. Under a correct model it's uniformly distributed between 0 and 1, so a warning at below 0.025 or above 0.975 fires by chance 5% of the time. Otherwise:

- **Below 0.025:** the points scatter more than their uncertainties explain. Possible causes are underestimated per-point uncertainties, unsettled points, or curvature the line can't follow.
- **Above 0.975:** the points scatter suspiciously little. The per-point uncertainties are probably overestimated (e.g. the quantisation floor) or the points are correlated.

**The statistical uncertainty.** $u(R_0)$ from 3.4.1 is how well the random scatter in the nine points pins down the intercept: the per-point Type A uncertainties propagated through the fit. It assumes those per-point uncertainties are right, and $\chi^2_\nu$ tests that assumption.

The Birge ratio is $\sqrt{\chi^2_\nu}$, the ratio of the observed scatter to the predicted scatter. Setting $u_{fit} = u(R_0)\cdot\max(1, \sqrt{\chi^2_\nu})$ is equivalent to inflating every point's uncertainty by a common factor until $\chi^2_\nu = 1$. It covers scatter the per-point uncertainties missed (e.g. imperfect settling). It's never applied as a deflation, because with only 7 degrees of freedom a small $\chi^2_\nu$ often happens by chance, and it's no evidence that the points are better than their stated uncertainties. It also assumes the extra scatter is random. Systematic curvature shows up as a small p-value, which is why that warning exists alongside the inflation.

#### 3.4.3 Oscilloscope DC gain

Model: every voltage is read as $(1+\varepsilon)V_{true}$ with a single unknown $\varepsilon$, which is why the vertical scale is held fixed. Reversal doesn't cancel it. It scales every $R_i$ by $(1+\varepsilon)$, and since $x$ comes from the current, the fitted intercept scales by exactly $(1+\varepsilon)$. Treating $\varepsilon$ as rectangular on $[-\varepsilon_{max}, \varepsilon_{max}]$:

$$u_{Vgain} = R_0\,\varepsilon_{max}/\sqrt{3}$$

What the data sheet actually specifies: "±1% (>5 mV/div, FullScale)" is a bound on the _absolute_ error. At any reading on a given scale, the error is at most $0.01 V_{FS}$, where $V_{FS}$ = 8 div × scale (note [4]). That's 0.8 mV at 10 mV/div. Expressing a spec as a percentage of full scale, rather than of reading, is what lets it cover error components that don't scale with the reading. It doesn't say the error is a pure gain.

The pure-gain model above is therefore an assumption, layered on top of the data sheet. It's the standard physical model of a front end, but it isn't guaranteed. Under the assumption, the bound on $\varepsilon$ depends on what the full scale refers to:

At 10 mV/div the screen shows −40 mV to +40 mV, with 0 V on the centre line, and "full scale" is the whole 80 mV. But a single DC reading can be at most 40 mV from zero, half of full scale. So what is the 1% of 80 mV (0.8 mV) a bound on?

- **A single reading (conservative).** Read literally, any one reading is within 0.8 mV. For a pure gain error $\varepsilon$ the error grows with the reading, so the worst case is the largest possible reading, 40 mV, where the error is $\varepsilon\times$ 40 mV. Requiring that to be at most 0.8 mV gives $\varepsilon_{max} = 0.8/40 = 2\%$.
- **The difference between two readings (the likelier meaning).** The only way to use all 80 mV is to compare a reading near the top of the screen with one near the bottom, say +40 mV and −40 mV, whose difference is 80 mV. That's also how DC gain is usually verified: apply a known positive voltage and a known negative voltage and check the difference between the readings, which removes the scope's offset. On this reading, the spec says the measured difference is within 1% of 80 mV. A gain error $\varepsilon$ makes the measured difference $(1+\varepsilon)\times$ 80 mV, so $\varepsilon_{max} = 1\%$.

The measurement here is exactly that second kind. $V = (V_P - V_N)/2$ is half the difference between a positive and a negative reading, taken for the same reason (to remove offsets). So the second reading fits it naturally.

`VOLTAGE_GAIN_BOUND` defaults to the conservative 0.02, which gives $u_{Vgain} \approx 1.15\%$. The span reading's 0.01 (0.58%) is plausible but isn't what the data sheet literally guarantees. Note that 0.02 is the worst case _given_ the pure-gain assumption, not the worst case overall. A non-proportional error of the same size would be worse (see below). It doesn't depend on the particular voltages measured. Scales of 5 mV/div or less are never used, because the scale list starts at 10 mV/div.

If the assumption is dropped and only the literal bound is used, the error in $V_i$ is an arbitrary function of $V$ within ±0.8 mV. Anything even in $V$ cancels on reversal, but whatever flips sign with $V$ survives and doesn't scale $R_0$ cleanly. For the test data in "Tests" (about 10–30 mV):

- a constant, sign-following error of 0.8 mV shifts $R_0$ by about 7.4%;
- the worst possible error shape shifts it by about 9.4%.

The data sheet alone therefore can't bound $R_0$ better than several percent. Tightening this requires measuring the scope's actual response at the chosen scale (see the caveat under "DC gain accuracy" in the error-source appendix). A measured gain and linearity would replace $\varepsilon_{max}$ with a much smaller calibration uncertainty. Until then, results rest on the pure-gain assumption, and the results display should say so.

#### 3.4.4 Power supply readback gain

Model: $I_{meas} = (1+g)\,I_{true}$ with a single unknown $g$ for the whole run, $\lvert g\rvert \le g_{max}$. The voltage is unaffected, so:

- $R_{meas} = V/I_{meas} = R_{true}/(1+g)$;
- $x_{meas} = I_{meas}^2 = (1+g)^2 x_{true}$, so $x_{true} = x_{meas}/(1+g)^2$.

Substitute both into the true line $R_{true} = R_0 + b\,x_{true}$:

$$R_{meas} = \frac{R_{true}}{1+g} = \frac{1}{1+g}\left(R_0 + \frac{b\,x_{meas}}{(1+g)^2}\right) = \frac{R_0}{1+g} + \frac{b}{(1+g)^3}\,x_{meas}$$

The measured points still lie on a straight line in $x_{meas}$.

The fitted intercept is exactly $R_0/(1+g) \approx R_0(1-g)$; the $x$ rescaling only changes the slope. Treating $g$ as rectangular:

$$u_{Igain} = R_0\,g_{max}/\sqrt{3}$$

With the default $g_{max}=0.0015$ this is about 0.09% of $R_0$.

#### 3.4.5 Power supply readback offset (corner analysis)

Model: $I_{meas} = I_{true} + \delta$ with a single unknown $\delta$, $\lvert\delta\rvert \le \delta_{max}$.

Reversal does **not** cancel this. The supply reads back a positive current in both relay states, so $\delta$ is the same in both polarities and survives $I = (\bar I_P + \bar I_N)/2$.

It is also not a simple scale factor on $R_0$. It distorts each $R_i$ by about $-\delta/I_i$, which is largest at low current, and the extrapolation to $I = 0$ amplifies it.

Procedure, for each $\delta \in \{-\delta_{max}, +\delta_{max}\}$:

1. For every point, set $I'_i = I_i - \delta$ (same $u(I_i)$), recompute $R'_i = V_i/I'_i$ and $x'_i = I_i'^2$, and propagate uncertainties exactly as in 3.3.
2. Refit exactly as in 3.4.1 to get $R_0(\delta)$.

Then $h = \lvert R_0(+\delta_{max}) - R_0(-\delta_{max})\rvert/2$ is the half-width of the limit of error. $\delta = 0$ isn't needed: it's the nominal fit from 3.4.1, and because $R_0$ is monotonic (very nearly linear) in $\delta$ over this range, the extremes are always at the two ends. The standard uncertainty (rectangular $\delta$) is

$$u_{offset} = h/\sqrt{3}$$

With the default setpoints this term is expected to dominate the budget (a few percent of $R_0$).

#### 3.4.6 Combined uncertainty

The reported value is the nominal fit $R_0$ from 3.4.1 (i.e. $\delta = 0$). The combined standard uncertainty, treating the four sources as independent, is

$$u_c(R_0) = \sqrt{u_{fit}^2 + u_{Vgain}^2 + u_{Igain}^2 + u_{offset}^2}$$

$u_c$ is the standard deviation of the probability distribution for the true value, so for a roughly normal distribution $R_0 \pm u_c$ covers about 68%. The expanded uncertainty $U = k\,u_c$ widens that to a stated coverage; with the coverage factor $k = 2$, $R_0 \pm U$ covers about 95%. It isn't stored, since it's just $2u_c$; 3.5 displays it with its $k$. The coverage is approximate, because the budget is dominated by a rectangular Type B term and the combined distribution isn't quite normal.

#### 3.4.7 Correction to 20 °C

$R_0$ is the resistance at the filament's unheated temperature $T_f$. It is corrected to the reference temperature $T_{ref} = 20$ °C using tungsten's temperature coefficient $\alpha$:

$$R_{20} = \frac{R_0}{1+\alpha\,(T_f - T_{ref})} = R_0\,f, \qquad f = \frac{1}{1+\alpha\,\Delta T}$$

The inputs come from 3.0 and 3.2:

- $T_f$ is the mean of the start and end chamber temperatures.
- Its bound is $a_T = $ `THERMOMETER_BOUND_KELVIN` $+ \lvert T_{end} - T_{start}\rvert/2$. The linear sum is deliberately conservative: it covers the thermometer and placement, plus drift over the run.

The Rust side computes $T_f$ and $a_T$ and passes them to the script in `ColdResistanceFitParameters`.

**Notation.** $u(T)$ and $u(\alpha)$ are the standard uncertainties of the _inputs_, in kelvin and per kelvin. $u_T$ and $u_\alpha$ are their _contributions_ to the uncertainty of $R_{20}$, in ohms. Each contribution is the input's uncertainty times the magnitude of $R_{20}$'s sensitivity to that input, $\lvert\partial R_{20}/\partial x\rvert$, which converts kelvin (or per kelvin) into ohms. This is the same pattern as $u_{Vgain}$ and the other contributions in 3.4.3–3.4.5.

Treating $T_f$ and $\alpha$ as rectangular, $u(T) = a_T/\sqrt3$ and $u(\alpha) = a_\alpha/\sqrt3$.

$R_{20} = R_0 f$ depends on three independent inputs: $R_0$, $T_f$ and $\alpha$. The partial derivatives are:

- $\partial R_{20}/\partial R_0 = f$;
- $\partial R_{20}/\partial T_f = R_0\,\partial f/\partial T_f = -R_0\,\alpha f^2 = -R_{20}\,\alpha f$;
- $\partial R_{20}/\partial\alpha = -R_0\,\Delta T f^2 = -R_{20}\,\Delta T f$.

So the contributions are

$$u_T = R_{20}\,\alpha f\,u(T), \qquad u_\alpha = R_{20}\,\lvert\Delta T\rvert f\,u(\alpha)$$

The derivatives' signs drop out because each term enters the combination squared. Signs only matter for covariance terms between correlated inputs, and $T_f$, $\alpha$ and $R_0$ are independent.

These combine with the law of propagation of uncertainty (GUM equation 10). For independent inputs $x_i$, $u_c^2(y) = \sum_i \left(\frac{\partial y}{\partial x_i}\right)^2 u^2(x_i)$. The $R_0$ term is its sensitivity $f$ times its uncertainty $u_c(R_0)$, the combined uncertainty from 3.4.6:

$$u_c(R_{20}) = \sqrt{\left(f\,u_c(R_0)\right)^2 + u_T^2 + u_\alpha^2}$$

Since $R_{20} = f R_0$, the first term keeps $R_0$'s _relative_ uncertainty unchanged when it's carried over to $R_{20}$. With $f \approx 0.98$ at 25 °C it barely differs from $u_c(R_0)$. Dividing through by $R_{20}$ gives the same thing in relative terms:

$$\left(\frac{u_c(R_{20})}{R_{20}}\right)^2 = \left(\frac{u_c(R_0)}{R_0}\right)^2 + \left(\alpha f\,u(T)\right)^2 + \left(\lvert\Delta T\rvert f\,u(\alpha)\right)^2$$

As for $R_0$, the expanded uncertainty $U = 2\,u_c(R_{20})$ is displayed but not stored.

The correction treats everything between the sense points as tungsten. If a significant part of $R_0$ is weld or post material with a different $\alpha$, the correction is slightly off. Because the correction itself is only a few percent, the error in it is a small fraction of that.

### 3.5 Showing the result

Record the analysis and save. Show:

- $R_{20} \pm u_c(R_{20})$ and $U = 2u_c(R_{20})$ ($k = 2$) as the headline result, with $R_0 \pm u_c$ at $T_f$ alongside it;
- the correction's terms ($u_T$ and $u_\alpha$);
- each budget component, in Ω and as a percentage of $R_0$;
- $b$, the slope of the fit in Ω/A². From 'Why extrapolating to $I^2 = 0$ gives $R_0$', $b = R_0 c = \alpha R_0^2/G$, so it measures how strongly the filament self-heats. $G = \alpha R_0^2/b$ is a first estimate of the filament's effective thermal conductance to its mount, which will be useful for the thermal time constant work ($\tau = C/G$). Show $G$ too. The fit script computes it with its uncertainty, using the covariance between $R_0$ and $b$ from 3.4.1 (Birge-scaled as in 3.4.2) and the uncertainty in $\alpha$. If $b$ isn't clearly positive, as under `--mock`, where there's no self-heating, there's no estimate and the fit warns instead;
- $\chi^2_\nu$ and its p-value;
- any warnings.

## 4. Spinning down and finishing

Unchanged. Abort handling is already covered by ARCHITECTURE.md §16: every `Err`, including the new `ProcedureError::Check`, ends the run and `enter_safe_state` runs. Its current step to 0 A (rather than a ramp) is harmless at ≤300 mA.

# Tests

## Python (`test_cold_resistance.py`)

Tests 1–3 share a fixture. It uses the default setpoints $I_i$ = 0.100, 0.125, …, 0.300 A and noiseless voltages $V_i = R(I_i)\,I_i$, with $R(I) = R_0 + bI^2$, $R_0 = 0.096$ Ω and $b = 0.096 \times 0.02/0.09 \approx 0.021333$ Ω/A². That value of $b$ makes the resistance 2% above $R_0$ at 300 mA. Every point has $u(V_i) = 1$ µV and $u(I_i) = 20$ µA, propagated to $u(R_i)$ as in 3.3. The absolute uncertainties are the same at every point, but $u(R_i)/R_i = \sqrt{(u(V_i)/V_i)^2 + (u(I_i)/I_i)^2}$ shrinks as the voltage and current grow. So $u(R_i)$ falls from about 21.7 µΩ at 100 mA to about 7.3 µΩ at 300 mA, and the 300 mA point carries about 9× the weight of the 100 mA point.

1. **Recovery:** the fit must return $R_0 = 0.096000$ Ω and $\chi^2 = 0$. The data lies exactly on a line, so the weights don't matter.
2. **Gain invariance:** scaling every $V$ by $(1+\varepsilon)$ must scale $R_0$ by exactly $(1+\varepsilon)$. Scaling every $I$ by $(1+g)$ must scale $R_0$ by exactly $1/(1+g)$.
3. **Offset corners:** on the fixture with $\delta_{max} = 5$ mA, the corners are $R_0(+5\ \text{mA}) \approx 0.099849$ Ω and $R_0(-5\ \text{mA}) \approx 0.092317$ Ω, so $h \approx 0.003766$ Ω and $u_{offset} \approx 0.002174$ Ω (≈2.3% of $R_0$). $R_0(+5\ \text{mA})$ means $I'_i = I_i - 0.005$. Once the currents are shifted the data is no longer a line, so these values depend on the weights. They were computed by propagating the fixture's uncertainties for the shifted currents and fitting with `curve_fit(..., absolute_sigma=True)`, exactly as 3.4.5 specifies. Check to about $10^{-5}$ relative.
4. **Temperature correction:** $R_0 = 0.096$ Ω at $T_f = 25$ °C with $\alpha = 0.0045$ gives $R_{20} \approx 0.093888$ Ω. With $a_T = 1$ K, $u_T \approx 2.39\times10^{-4}$ Ω; with $a_\alpha = 0.0003$, $u_\alpha \approx 7.95\times10^{-5}$ Ω. At $T_f = 20$ °C, $R_{20} = R_0$ and $u_\alpha = 0$.
5. **Quantisation floor:** a current with a standard error of 0 must come out with $u = \Delta_I/\sqrt{12}$.
6. **Fit cross-check:** on noisy synthetic data with unequal uncertainties, the fit's $R_0$, $b$, $u(R_0)$ and $u(b)$ match the closed-form weighted least squares, computed in the test itself, to within $10^{-5}$ relative (`curve_fit` is iterative, and its uncertainties agree to about $5\times10^{-7}$). This catches a missing `absolute_sigma=True`, which would scale the uncertainties by $\sqrt{\chi^2_\nu}$. With $w_i = 1/u(R_i)^2$:

   $$S=\sum w_i,\ S_x=\sum w_i x_i,\ S_y=\sum w_i R_i,\ S_{xx}=\sum w_i x_i^2,\ S_{xy}=\sum w_i x_i R_i,\ \Delta = S\,S_{xx}-S_x^2$$

   $$R_0=\frac{S_{xx}S_y-S_xS_{xy}}{\Delta},\quad b=\frac{S\,S_{xy}-S_xS_y}{\Delta},\quad u(R_0)^2=\frac{S_{xx}}{\Delta},\quad u(b)^2=\frac{S}{\Delta}$$

7. **Sign check:** a positive-polarity mean that isn't positive, or a negative-polarity mean that isn't negative, exits non-zero with an error naming which.
8. **Schemas:** each script's output validates against its output schema. Input that violates the input schema (a missing field, an unknown field, a string where a number belongs) exits non-zero with the validation error on stderr.

## Rust

- `ramp_to` produces the expected setpoint sequence, ending exactly on the target.
- `change_polarity` disables the output before switching the relays and leaves the setpoint unchanged (against the mock, which refuses to switch relays with the output on).
- Under `--mock`, the procedure runs end to end, and the results file contains a `cold_resistance` with nine points and an analysis whose $R_0$ is 0.096 Ω.
  - The test answers the prompts itself, through the step tree's responders, as the application would.
  - It runs on tokio's paused clock, so the ramps and settles take no real time, and the whole test takes a few seconds rather than minutes. For that, timing in the procedure uses `tokio::time::Instant`, and `run_script` has no timeout in test builds: tokio jumps a paused clock to the next timer whenever every task is waiting, including on a script, so the timeout would fire at once.
  - It needs the Python virtual environment, which CI's root job creates.

# Acceptance criteria

These are in addition to ARCHITECTURE.md §20.

- A clipped reading, including one where only VMAX or VMIN is out of range, during the vertical-scale search increases the scale and never ends the run as a hardware failure.
- A clipped voltage after the scale is chosen, an OCP trip, a supply leaving constant-current mode, or a pressure above `FILAMENT_ABORT_PRESSURE_MBAR` each end the run with a `Check` error step, followed by the usual cleanup.
- The relays are only ever switched with the output off. Setpoint changes only ever happen in `RAMP_STEP_AMPS` steps. A polarity change turns the output off and back on at the same setpoint.

# Other filaments

Every systematic term in the budget is relative, so none of them depends on $R_0$:

- the scope gain term scales $R_0$ by $(1+\varepsilon)$;
- the current gain term scales $R_0$ by $1/(1+g)$;
- the offset term distorts each point by about $\delta/I$, which depends on current, not resistance.

What does depend on $R_0$ is the signal size and the self-heating:

- **Much smaller $R_0$ (tens of mΩ):** the voltages shrink to a few mV, a small fraction of a division even at 10 mV/div. The systematic terms are unchanged, but noise per sample is a larger fraction of the signal, so $u_{fit}$ grows. Don't drop below 10 mV/div to compensate, because the gain spec is only "typical" there. Raise the currents instead. Self-heating is $I^2R$, so a smaller $R_0$ tolerates proportionally more current for the same temperature rise, and larger currents also shrink the $\delta/I$ offset term. Keep the setpoints below `MAXIMUM_HEATING_CURRENT_AMPS`, raising it deliberately if needed.
- **Much larger $R_0$ (ohms):** the voltages grow, which is easy to measure; the scale search simply lands on a coarser scale. The supply's limits don't need to change. At 300 mA the 2 V limit covers about 5 Ω including leads and relays, and it's the total circuit voltage that must stay under the limit. Self-heating grows in proportion to $R_0$, though, so the $R$ vs $I^2$ line develops curvature. That would show as a large $\chi^2_\nu$, and the extrapolation becomes model-dependent. Lower the current range so the heating stays similar, which means scaling the currents by about $1/\sqrt{R_0 / 0.1\ \Omega}$.

In both cases, what matters is roughly constant heating power ($I^2R$ up to about 10 mW) at the top setpoint, and a 3:1 ratio between the largest and smallest setpoints.

# Appendix: the supply's current readback

The DP900 programming guide doesn't say how `:MEASure:CURRent?` works. It could take a new measurement for each query, or return the latest result of a measurement the supply repeats on its own schedule. This was tested on the DP932E with throwaway binaries (since deleted), with a 10 Ω resistor across CH1 at 100–110 mA.

**Each query takes a new measurement of about 180 ms.** Each query's round trip was timed ten times after a second of idle, and ten times back to back:

| Query                   | After idle     | Back to back   |
| ----------------------- | -------------- | -------------- |
| `*IDN?`                 | 1.1–19.4 ms    | 0.8–2.4 ms     |
| `:SOURce1:CURRent?`     | 1.1–1.9 ms     | 0.8–1.3 ms     |
| `:MEASure:CURRent? CH1` | 173.1–199.5 ms | 173.6–212.2 ms |

- The USB link and the supply's command handling are fast: the queries that don't measure answer in a few milliseconds.
- `MEASure?` takes about 180 ms even after a second of idle.
  - If it returned a stored value, the reply would come in a few milliseconds.
  - If it waited for the next result of a repeating measurement, the wait would land anywhere in that cycle and some replies would be quick.
  - The fastest was 173 ms, so each query starts its own measurement.
- So consecutive readings are always independent, and there's no update interval to wait out between samples.

**The current is steadier than the readback resolution.** Querying back to back at a constant 100 mA for 30 s, the value changed only twice. That's why every sample at a point usually reads the same, and why the $\Delta_I/\sqrt{12}$ quantisation floor (3.3) matters.

**Measurements that follow each other too closely hold up setpoint changes.** Each trial settled at 100 mA, wrote 110 mA, then queried the current until it showed the step, with a pause between queries.

- With a 20 ms pause, the step still hadn't shown after 2 s. Once the queries stopped, the supply applied it.
- With a 100 ms pause, the step took about 0.8 s on average to show.
- With a 200 ms pause, all 50 trials saw the step 0.55–0.63 s after the write, which is when the second reply arrives. The first reply, about 180 ms after the write, still showed 100 mA. No reply lay between 100 and 110 mA.

The supply presumably only applies a setpoint while it isn't measuring. Hence the 200 ms `MEASUREMENT_PAUSE` in `get_current`: it's the pause that was tested and shown to work.

# Appendix: error sources

For each source: where it comes from, how it's modelled and converted to a standard uncertainty, what it does to $R_0$, and how the procedure handles it.

**Terminology.** A _bound_ (or half-width) $a$ is a limit on an error: the error is assumed to lie within $\pm a$. It's what data sheets give ("±0.15%"), and it isn't a standard uncertainty. A _standard uncertainty_ $u$ is the standard deviation of the error's probability distribution, which is what gets propagated and added in quadrature. A bound is converted to a standard uncertainty with a Type B conversion below.

Two conversions recur:

- **Type A** (statistical): a mean of $n$ samples has standard uncertainty $s/\sqrt n$, the standard error.
- **Type B** (from a specification): a data-sheet bound $\pm a$ with no stated distribution is treated as rectangular (every value in the range equally likely), whose standard deviation is $a/\sqrt3$. This is the GUM default. When a width is given as a full width $\Delta$ rather than a half-width, as for a display resolution (the true value lies within $\pm\Delta/2$ of the reading), the same formula with $a = \Delta/2$ gives $u = \Delta/(2\sqrt3) = \Delta/\sqrt{12}$, or variance $\Delta^2/12$ (GUM F.2.2.1).

Systematic terms that are common to every point are applied to $R_0$ after the fit. They can't be added to each point's uncertainty before fitting, because they're perfectly correlated between points and the fit would treat them as independent scatter.

## Oscilloscope

**DC gain accuracy.**

- Source: data sheet, ±1% of full scale (8 div × scale) above 5 mV/div; ±2% "typical" at or below 5 mV/div.
- Model: every voltage is multiplied by the same $(1+\varepsilon)$, because the scale never changes during a run.
- Standard uncertainty: $\varepsilon_{max}/\sqrt3$ relative, with $\varepsilon_{max} = 0.02$ (3.4.3). That's 1.15%, or 0.58% under the span reading's 0.01.
- Effect on $R_0$: multiplies it by exactly $(1+\varepsilon)$, so $u_{Vgain} = R_0\varepsilon_{max}/\sqrt3$.
- Handling: added in quadrature after the fit. Not cancelled by reversal, since $(gV - (-gV))/2 = gV$. Scales at or below 5 mV/div are never used.
- Caveats: the data sheet doesn't say the error is a pure gain. That's the standard instrument model: the gain is set by the front-end attenuator and amplifier and the ADC reference, calibrated per range, and fixed apart from slow drift with temperature and age. The data sheet also has no linearity spec. A non-proportional error that flips sign with the input (a nonlinearity) would survive reversal and wouldn't scale $R_0$ cleanly, and read at its most pessimistic (up to 1% of full scale at any reading) the spec doesn't rule one out. The way to settle it is to measure: apply known voltages spanning ±10–30 mV (from a precision reference and divider, or checked with a DMM better than about 0.1%) at the chosen scale, and fit scope reading against true voltage. That gives the actual gain, far tighter than 1%, and the residuals show whether the error is linear.

**DC offset accuracy.**

- Source: data sheet, ±(0.1 div + 2 mV + 1.5% of the offset setting). The offset setting is 0 V, so the last term is zero. That's up to ±3 mV at 10 mV/div, comparable to the signal.
- Model: the same additive constant on every reading on a given scale.
- Effect on $R_0$: none after reversal, because $V = (V_P - V_N)/2$ removes anything that doesn't change sign with the current. Offset drift in the seconds between the two polarities doesn't cancel, but alternating which polarity goes first (3.2.2.1) makes it cancel on average rather than bias every point the same way. That scatter is covered by $u_{fit}$ and, if it's large, by the Birge inflation.
- Handling: cancelled. $V_{off}$ is logged per point as a diagnostic.

**Vertical resolution.**

- Source: 12 bits over 8 div, so one LSB is about 20 µV at 10 mV/div.
- Model: quantisation. VAVG averages the thousands of points in a 20 ms window, and noise dithers them, so the effective resolution is far finer than one LSB.
- Effect on $R_0$: negligible. Anything left is random and shows up in the standard error.
- Handling: none needed.

**Random noise** (front-end noise, residual pickup).

- Model: random variation between samples.
- Standard uncertainty: Type A, the standard error of the $n$ VAVG samples in each polarity.
- Effect on $R_0$: propagates to $u(R_i)$, which sets the fit weights and $u_{fit}$.
- Handling: the 20 MHz bandwidth limit and the 20 ms (one mains cycle) window reduce it before it's measured.

**Input loading.**

- Source: 1 MΩ ± 1% input impedance with a 1× probe.
- Model: the scope draws current from the sense pair, whose source resistance is the filament plus the sense leads (well under 1 Ω).
- Effect on $R_0$: about $10^{-6}$ relative.
- Handling: none needed. A 10× probe would instead add its attenuation tolerance as another multiplicative term like the gain.

**Timebase accuracy** (±25 ppm) and mains frequency variation (±0.15 Hz or so).

- These make the 20 ms window slightly different from an exact mains cycle, so a small fraction of any pickup survives each sample.
- It's random from sample to sample, so it's covered by the standard error.
- Handling: none needed.

## Power supply (current readback)

**Readback gain.**

- Source: data sheet, the 0.15%-of-output part of ±(0.15% + 5 mA), at 25 °C ± 5 °C within 12 months of calibration.
- Model: every reading is multiplied by $(1+g)$.
- Standard uncertainty: $g_{max}/\sqrt3$ relative, about 0.09%.
- Effect on $R_0$: divides it by exactly $(1+g)$ (3.4.4), so $u_{Igain} = R_0 g_{max}/\sqrt3$.
- Handling: added in quadrature after the fit.

**Readback offset.**

- Source: data sheet, the 5 mA part of the same spec.
- Model: every reading has $\delta$ added, the same in both polarities, because the supply always sources positive current and the relays do the reversal.
- Effect on $R_0$: not a scale factor. Each point is distorted by about $-\delta/I_i$, most at low current, and the extrapolation to $I = 0$ amplifies that.
- Standard uncertainty: the corner analysis (3.4.5) gives the half-range $h$ of $R_0$ over $\delta = \pm\delta_{max}$, and the standard uncertainty is $h/\sqrt3$. That's about 2–3% with the default setpoints (it depends slightly on the per-point uncertainties, which set the weights), and it's the dominant term.
- Handling: added in quadrature after the fit.

**Temperature coefficient.**

- Source: data sheet, 0.01% + 2 mA per °C for current on CH1/CH2.
- Model: widens the gain and offset bounds for each degree outside 20–30 °C (3.0). The data sheet doesn't say how the coefficient combines with the accuracy spec, so this follows the usual convention.
- Effect on $R_0$: the offset part matters and the gain part doesn't. At 15 °C the offset bound triples to 15 mA, and so does $u_{offset}$.
- Handling: through the widened bounds. The oscilloscope has no equivalent adjustment, because its data sheet gives no temperature coefficient and guarantees its specs across its whole 0–50 °C operating range (after 30 minutes' warm-up).

**Readback resolution.**

- Source: 0.1 mA with HIRES.
- Model: quantisation. If the true current is steadier than one step, every sample reads the same, the standard error is 0, and the true value is anywhere within ±0.05 mA.
- Standard uncertainty: $\Delta_I/\sqrt{12}$, a rectangular distribution with full width $\Delta_I$.
- Effect on $R_0$: through $u(I)$ and hence $u(R_i)$.
- Handling: added in quadrature to each polarity's standard error (3.3).

**Readback noise, ripple and regulation drift.**

- Model: random.
- Standard uncertainty: Type A, the standard error of the current samples. It's only valid if the samples are independent, which they are: each `:MEASure:CURRent?` takes a new measurement (see "Appendix: the supply's current readback").
- Handling: none needed beyond the standard error. When the current is steadier than the resolution, the standard error is 0 and the quantisation floor above takes over.

**Programming accuracy** (0.2% + 5 mA).

- Irrelevant: the setpoint only chooses where to measure, and the readback is what's used.

## The rig and the filament

**Thermoelectric (Seebeck) EMFs** at the junctions between dissimilar metals.

- Model: additive voltages that don't reverse with the current.
- Handling: cancelled by reversal, like the scope offset. Drift between polarities cancels on average through the alternating order.

**Lead, relay, feedthrough and contact resistance.**

- Handling: excluded by four-terminal sensing. The sense pair carries almost no current (1 MΩ input), so these resistances drop almost nothing across the voltage measurement.

**Self-heating.**

- Model: the measured $R$ rises with $I^2$. The linear fit $R = R_0 + bI^2$ extrapolates it away.
- Effect on $R_0$: at about 10 mW the curvature from nonlinear heating is negligible. If it weren't, it would show as a large $\chi^2_\nu$.
- Handling: the fit, with $\chi^2_\nu$ as the check.

**Incomplete thermal settling.**

- Model: a point measured before the filament has settled reads slightly low.
- Handling: the settle criterion reduces it. What's left is random between points, thanks to the shuffled order. It appears in $\chi^2_\nu$ and is covered by the Birge inflation of $u_{fit}$.

**Filament temperature and the correction to 20 °C.**

- Source: tungsten's resistance rises about 0.45% per kelvin near room temperature. So the extrapolated $R_0$ is the resistance at the filament's unheated temperature, and two filaments measured 5 °C apart differ by about 2% for that reason alone.
- Model: $R_{20} = R_0/(1+\alpha\,\Delta T)$ (3.4.7).
- Standard uncertainty, from the filament temperature: $a_T/\sqrt3$, with $a_T$ covering the thermometer, how far the filament sits from the flange, and drift over the run. From $\alpha$: $a_\alpha/\sqrt3$.
- Effect on $R_{20}$: $u_T \approx 0.26\%$ for $a_T = 1$ K. $u_\alpha$ grows with $\lvert\Delta T\rvert$: about 0.09% at 5 K from 20 °C, and 0 at 20 °C.
- Handling: added in quadrature to $f\,u_c(R_0)$. Drift during the run is also partly randomised by the shuffled order.

## The fit

**Statistical uncertainty.**

- Source: the per-point Type A uncertainties propagated through the fit. It's `sqrt(pcov[0, 0])` from `curve_fit` with `absolute_sigma=True` (3.4.1), inflated by $\sqrt{\chi^2_\nu}$ when that's above 1 (3.4.2).
- Handling: added in quadrature with the systematic terms.

**Uncertainty in $x = I^2$.**

- Neglected in the fit, because its effect, $b\,u(x_i)$, is a few percent of $u(R_i)$.
- Handling: checked by the logged ratio (3.4.1).
