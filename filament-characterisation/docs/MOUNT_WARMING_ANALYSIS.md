# Mount warming: analysis and predictions

How much the filament's mount warms during a hot sweep, worked out from the first two sweeps on real hardware, and the prediction for a sweep to 0.9 A made before running it. This is an analysis record, not a design document: nothing here changes the procedure.

The model's code is `characteriser/python/mount_warming.py`. The prediction for 0.9 A was made with the script in the appendix, which isn't in the repository otherwise.

## Summary

- **The mount warms measurably during a sweep.** At every setpoint, the resistance on the way down is higher than on the way up.
- **A first-order model fits it well.** Each run gives the same parameters within 5%: k ≈ 0.14 mΩ of extra resistance per mW of filament power at equilibrium, with a time constant τ ≈ 2 minutes.
- **Peak warming at the clamp, from the model:**

  | Run | Peak extra resistance | Peak rise, W-Re α | Peak rise, pure W α |
  | --- | --- | --- | --- |
  | 0.6 A | 4.27 mΩ | 14.1 K | 9.7 K |
  | 0.8 A | 7.62 mΩ | 25.0 K | 17.2 K |
  | 0.9 A (predicted) | about 11.2 mΩ (10.7–12.0) | about 37 K (35–40) | about 25 K (24–27) |

- **The epoxy softens at about 80 °C** (see `5_HOT_SWEEP.md`). From a 20 °C start, the 0.9 A prediction is about 57 °C at the clamp.
- **The biggest uncertainty in the temperatures is α,** resistance's temperature coefficient, which isn't known for this wire to better than about ±20%. The resistances themselves are much better known.

## The data

Two hot sweeps of filament 1, in vacuum, on 21 October 2026. Their results files aren't in the repository, so the numbers needed to follow the analysis are summarised here, and the appendix includes the data the prediction uses. Runs are identified by their `started_at` timestamp, which is also in the results file's name.

| Run | Started | Maximum | I² step | Points | First point | Top point | Last point |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1792550316 | 13:38 | 0.6 A | 0.02 A² | 37 | 105 s | 500 s | 850 s |
| 1792553030 | 14:23 | 0.8 A | 0.05 A² | 27 | 106 s | 411 s | 677 s |

Times are on each run's own clock, `time_seconds` in the results, which is the middle of the point's sampling window. Before the first point the output is off while the scope's offsets are read, so the filament is unpowered.

The second run started 45 minutes after the first, about 30 minutes after the first finished. That's about 14τ, so the mount had cooled back to room temperature.

**Timing.** Points were 20.7 s apart on average in the 0.6 A run and 22.0 s in the 0.8 A run, 21.2 s across both. Each interval is about 15 s of fixed overhead plus the settle time. Settles were usually 4–6 s, but up to 14 s, mostly at higher currents. A change of the scope's vertical scale adds about 8 s. Within a point, the 20 samples take about 8 s, but they aren't timestamped individually.

**Power.** Each point's power is P = I·V from its analysis. It reached 39 mW at 0.6 A and 78 mW at 0.8 A.

## What the up/down difference tells us

The sweep measures each setpoint twice, once going up and once coming down, except the top point. At a given current, the filament itself settles within a second, so it's in the same state on both visits. Any difference between R_down and R_up at the same setpoint must come from something slower that changed between the visits. The mount is the obvious candidate: its temperature sets the temperature at the ends of the wire.

Taking the difference at the same current also cancels everything that depends only on current:

- **the filament's own heating,** the I² dependence the sweep is measuring;
- **a small low-current voltage error,** δ/I, which made the 100 mA resistances 2.5–4 mΩ low. δ is about −0.41 mV in the first run and −0.25 mV in the second, so it varies from run to run. Polarity reversal would cancel it, but the sweep doesn't reverse.

So `R_down − R_up` is a clean measurement of the mount's effect, with uncertainties of 22–93 µΩ, from the two resistances' statistical uncertainties combined in quadrature.

**The catch: it's the warming between the visits, not since the start.** Near the top of the sweep, a setpoint's two visits are only a minute or two apart, so the difference is small even though the mount is near its hottest. At the bottom, the up visit happened in the first minute, before the mount had warmed, so the difference is close to the full warming. Any warming that happened before the up visit cancels out of the difference, so the data alone can't give the absolute warming. A model is needed for that.

## The model

### The equation

The mount is treated as a single first-order lag driven by the filament's power:

$$\frac{dX}{dt} = \frac{k\,P(t) - X}{\tau}$$

- X is the extra filament resistance caused by the mount's warming, in ohms. It's proportional to the temperature rise at the clamp (see "Converting to temperature").
- P(t) is the filament's power, in watts.
- k is X's equilibrium value per watt of steady power, in Ω/W. Numerically this is the same as mΩ/mW.
- τ is the time constant, in seconds.
- X = 0 at the start of the run, i.e. the mount starts at room temperature.

The equation is linear: the response to a sum of power inputs is the sum of the responses to each.

### Solving it for a step

If power p switches on at time s and stays on:

$$X(t) = \begin{cases} 0 & t < s \\ k\,p\left(1 - e^{-(t - s)/\tau}\right) & t \ge s \end{cases}$$

X rises towards k·p, getting 63% of the way in one τ. It's 0 before the step: the mount can't respond to power it hasn't received yet.

### Solving it for one point: a pulse

Each point's power is applied over a slot, from `start` to `end`. Power on at `start` and off at `end` is a step of +p at `start` plus a step of −p at `end`. By linearity, the response is the difference of the two step responses, for t ≥ end:

$$X(t) = k\,p\left(1 - e^{-(t - \text{start})/\tau}\right) - k\,p\left(1 - e^{-(t - \text{end})/\tau}\right) = k\,p\left(e^{-(t - \text{end})/\tau} - e^{-(t - \text{start})/\tau}\right)$$

- At t = end this is the step response, cut off when the power stops.
- After that it decays as e^(−(t − end)/τ): the mount cools.

For example, with k = 0.144 Ω/W and τ = 130 s, 0.1 W for 20 s gives 2.05 mΩ at the end of the pulse (equilibrium would be 14.4 mΩ), falling to 0.76 mΩ 130 s later.

**The negative step isn't physical.** At the boundary between two points with powers p₁ and p₂, the code adds −p₁ (ending the first pulse) and +p₂ (starting the next). Together they're a single step of p₂ − p₁, the actual change in power. Writing the input as a staircase of steps, +p₁, then +(p₂ − p₁), and so on, gives exactly the same X(t). The pulse form is just convenient, because each point's contribution can be computed on its own.

### Solving it for a whole run

The power is taken as constant over each point's slot. Slot boundaries are halfway between neighbouring points' times. The first slot starts 10 s before the first point and the last ends 10 s after the last point, about half an interval (`EDGE_SECONDS`).

X at any time t is the sum of every slot's pulse response, which is exactly the convolution

$$X(t) = \int_{-\infty}^{t} \frac{k}{\tau}\, e^{-(t - s)/\tau}\, P(s)\, ds$$

evaluated slot by slot. In `mount_resistance`:

- **`end = min(edges[j + 1], t)`:** for the slot in progress at time t, only the power applied so far counts. With end = t, the off-step term is 0, as it should be for a step that hasn't happened.
- **`if end <= start: continue`:** slots that start after t haven't happened yet, so they contribute nothing.

Together these make sure the step formula is never evaluated before its step, where it would give nonsense.

**What the time-slot approximation ignores.** The current actually changes in a short ramp between setpoints, and the power while settling isn't exactly the sampled power. Both are small compared with the 20 s slots and a 2-minute τ.

### Fitting k and τ

For each up/down pair at the same setpoint, the model predicts X(t_down) − X(t_up). The fit chooses k and τ to minimise

$$\sum_{\text{pairs}} \left(\frac{X(t_\text{down}) - X(t_\text{up}) - (R_\text{down} - R_\text{up})}{u_\text{pair}}\right)^2$$

with `scipy.optimize.least_squares`, starting from 0.1 Ω/W and 155 s, the values from the earlier hold test in `4_TEMPERATURE_COMPENSATION.md`. τ is bounded below at 5 s so it can't collapse to 0. Parameter uncertainties come from the Jacobian, scaled by the reduced chi-squared.

Only the differences are fitted. The absolute X at each point comes from the fitted model, assuming it starts at 0.

### Fit results

| Run | Pairs | k (mΩ/mW) | τ (s) | Reduced χ² | dof |
| --- | --- | --- | --- | --- | --- |
| 0.6 A | 18 | 0.1441 ± 0.0010 | 130 ± 3 | 0.84 | 16 |
| 0.8 A | 13 | 0.1375 ± 0.0007 | 124 ± 3 | 1.29 | 11 |

- **Reduced χ² near 1** means the model fits about as well as the measurement uncertainties allow.
- **The two runs agree within 5%.** That's more than their statistical uncertainties, so it's a small model imperfection, but small for predictions.
- **k and τ are partly correlated.** A slower, stronger mount fits similarly to a faster, weaker one.
- **The model fitted to the 0.6 A run predicted the 0.8 A run's differences** before refitting, with the power doubled. That's the main evidence the model describes something real.

**Residuals.** Most are within 50 µΩ. The largest is the 0.6 A run's 141 mA point, 132 µΩ or 2.1 uncertainties. The 0.8 A run shows a slight pattern: above the model near the peak, below it in the middle, and above again at the low end. That hints at a second, slower heat path that a single time constant doesn't capture, which the hold test in `4_TEMPERATURE_COMPENSATION.md` also suggested. The misfit is only about one uncertainty, though.

### Model against the direct difference

The direct difference R_down − R_up and the model's X(t_down) − X(t_up) agree within 0.4 K at every setpoint, since that's what was fitted. The model's own values at each visit show what the difference can't see. In kelvin with the W-Re α, from the 0.8 A run:

| Setpoint (mA) | Direct difference | Model at up visit | Model at down visit |
| --- | --- | --- | --- |
| 775 | 5.3 | 18.5 | 23.5 |
| 632 | 15.4 | 9.1 | 24.6 |
| 500 | 17.3 | 4.0 | 21.3 |
| 224 | 14.0 | 0.3 | 14.1 |
| 100 | 12.3 | 0.0 | 12.0 |

The direct difference equals the absolute warming only at low setpoints, where the mount hadn't warmed by the up visit. Near the peak it's mostly a lower bound. The direct difference's largest value was 8.6 K in the 0.6 A run and 17.3 K in the 0.8 A run, at mid-currents. The model's peaks were 14.1 K and 25.0 K.

**What the absolute values rest on.** The model's warming before each up visit isn't measured. It depends on there being one time constant and on the mount starting at room temperature. The tight fit across many different up/down time gaps supports both assumptions, but they're still assumptions.

## Converting to temperature

Warming the whole wire uniformly by ΔT adds R₀·α·ΔT to its resistance, however hot it already is (see "Why the same amount is subtracted at every current" in `4_TEMPERATURE_COMPENSATION.md`). Raising the temperature where the wire is clamped lifts the whole wire's temperature profile by about the same amount, so

$$\Delta T = \frac{X}{R_0\,\alpha}$$

is the temperature rise at the clamp. Parts of the mount further from the posts will be cooler.

- **R₀ = 97.7 mΩ:** the filament's resistance at zero current with the mount at room temperature, extrapolated from the sweeps. R₀ came out the same in both runs, so there was no annealing between them. Its uncertainty matters much less than α's.
- **α = 0.0031 /K:** the best estimate for this tungsten–rhenium wire. Alloying lowers α compared with pure tungsten. It hasn't been measured on this filament.
- **α = 0.0045 /K:** pure tungsten's, used as a bound.

So 1 K is 0.303 mΩ with the W-Re α, or 0.440 mΩ with pure tungsten's. The two αs differ by about 45%, which is the largest uncertainty in every temperature here.

**Thermal resistance.** Dividing k by R₀α gives the clamp's rise per watt at equilibrium: 476 and 454 K/W for the two runs with the W-Re α, or 328 and 313 K/W with pure tungsten's. The rise is set by the power and by how long it's applied, compared with τ. Continuous operation at 100 mW would eventually warm the clamp by about 45 K, and at 1 W by about 450 K.

## Predicting a run before doing it

### Method

The model takes a time series of power and gives X(t). For a run that hasn't happened, the times and powers are estimated:

1. **Setpoints.** Apply the same rule as `hot_sweep_setpoints` in `procedure/characterisation/hot_sweep.rs`: 100 mA, then the multiples of the I² step above it up to the maximum, then back down. For 0.9 A (0.81 A²), the 0.05 A² step is the smallest that keeps the grid to 20 points or fewer. That gives 35 points:
   - up: 100, 224, 316, 387, 447, 500, 548, 592, 632, 671, 707, 742, 775, 806, 837, 866, 894, 900 mA;
   - then back down through the same setpoints without repeating 900 mA.

   This is the 0.8 A run's grid plus 806–900 mA.
2. **Power at each setpoint.** P = I²·R(I). R(I) is a quadratic in I² fitted to the 0.8 A run's up leg, then extrapolated. At 0.81 A² it gives R ≈ 130 mΩ, so the maximum power is about 106 mW.
   - The up-leg resistances include some mount warming, so this slightly overestimates the power, which errs on the safe side.
   - It's an extrapolation from 78 mW to 106 mW, 36% beyond the data.
3. **Timing.** A fixed interval between points, tried at 18, 21 and 24 s, bracketing the past runs' averages of 20.7 and 22.0 s.
4. **Model.** `mount_resistance` with each run's fitted k and τ, starting from X = 0, evaluated every second.
5. **Convert** the peak X to kelvin with both αs.

### Backtest

The same method was used to predict the 0.8 A run from the 0.6 A run's fit, using only synthetic setpoints, extrapolated power and fixed timing. Over 18–24 s per point, the predicted peak was 7.15–7.94 mΩ. At 21 s it was 7.59 mΩ, against 7.62 mΩ from the 0.8 A run's own fit, a 0.4% difference. That fit's peak is itself a model output, not a measurement, so this tests the prediction method's assumptions about setpoints, power and timing rather than the model itself.

### Prediction for 0.9 A

| k and τ from | 18 s per point | 21 s per point | 24 s per point |
| --- | --- | --- | --- |
| 0.6 A fit | 11.01 mΩ, 36.4 K | 11.56 mΩ, 38.2 K | 12.02 mΩ, 39.7 K |
| 0.8 A fit | 10.65 mΩ, 35.2 K | 11.18 mΩ, 36.9 K | 11.60 mΩ, 38.3 K |

Temperatures use the W-Re α. With pure tungsten's α they're 24.2–27.3 K.

- **Run length:** 612, 714 and 816 s from the first point to the last, for 18, 21 and 24 s per point.
- **When the peak comes:** at 387, 431 and 492 s after the first point. That's about 75 s after the 900 mA point, as in both past runs. The mount keeps warming while the current comes back down, until the power falls below about X/k.
- **What matters most:** timing matters about as much as which run's fit is used. Each extra 3 s per point adds about 1.5 K.

**Best estimate:** about 11.2 mΩ, 37 K at the clamp with the W-Re α (25 K with pure tungsten's), so about 57 °C from a 20 °C start.

### What could make the real rise higher

- **The mount not starting cold.** The model assumes room temperature at the first point. Leave the filament unpowered for at least 15 minutes, about 6τ, before the run.
- **A slower second heat path.** Hinted at by the residuals and the hold test. It would add warming that one time constant misses, and its effect grows with run length and power.
- **Longer settles near the top.** At 742 and 800 mA they reached 11–14 s. Longer intervals at the highest powers raise the peak more than the same extra time spread evenly.
- **Non-linearity.** The mount's conductivity and heat capacity change by only a few percent over 40 K, so the model should stay linear here. That gets less true the hotter the mount runs.

### Checking the prediction after the run

1. **Out-of-sample test.** Put the 0.9 A run's actual times and powers into `mount_resistance` with k and τ fixed from the earlier fits. Compare its predicted X(t_down) − X(t_up) with the measured R_down − R_up. Nothing is fitted to the new data, so this is a real test of the model.
2. **Refit.** Run `mount_warming.py` on the new file. If k and τ match the earlier runs, the model is still linear at this power. If k rises with power, it isn't, and further extrapolation needs a better model.
3. **Compare peaks.** The refitted peak against the prediction above, remembering that both are model outputs.

## Reproducing this

`mount_warming.py` fits the model to any hot sweep's results file. Run it from `characteriser/python/`, using the virtual environment described in `characteriser/README.md`, with the path of one or more results files:

```sh
.venv/bin/python mount_warming.py ../out/characterisation-<started_at>.json
```

The characteriser writes results to `characteriser/out/`, which isn't committed, so this needs a run's results file. The fits above came from runs 1792550316 and 1792553030.

For each run, it prints:
- k and τ with their uncertainties, and the reduced χ²;
- the peak warming for both αs;
- a table with each setpoint's direct difference, the model at the up and down visits, the model's difference, and the pair's uncertainty.

There's also an interactive chart of the model against the data, made during the analysis, at https://claude.ai/artifact/X2XZYmUguafAhtmkmzzcJf. It's private unless shared.

## Appendix: the prediction script

Save it in `characteriser/python/` and run it from there with `.venv/bin/python`, without `-I`, so it can import `mount_warming`. It doesn't need any results files: the data it uses from the 0.8 A run is included. Change `MAXIMUM` for another maximum, and `FITS` and `UP_LEG` once there are more runs.

```python
import numpy as np

from mount_warming import ALPHA_PER_KELVIN, EDGE_SECONDS, R0_OHMS, mount_resistance

MAXIMUM = 0.9
FITS = {"0.6 A": (0.1441, 129.8), "0.8 A": (0.1375, 124.3)}  # k (Ω/W), τ (s)

# The 0.8 A run's up leg: each point's measured current (A) and resistance (mΩ).
UP_LEG = [
    (0.10028, 95.429),  # 100 mA
    (0.22330, 97.566),  # 224 mA
    (0.31622, 99.157),  # 316 mA
    (0.38724, 100.740),  # 387 mA
    (0.44719, 102.339),  # 447 mA
    (0.50026, 103.905),  # 500 mA
    (0.54710, 105.651),  # 548 mA
    (0.59120, 107.653),  # 592 mA
    (0.63213, 109.627),  # 632 mA
    (0.67014, 111.667),  # 671 mA
    (0.70715, 113.960),  # 707 mA
    (0.74113, 116.490),  # 742 mA
    (0.77411, 118.894),  # 775 mA
    (0.80020, 121.408),  # 800 mA
]


def setpoints(maximum):
    """The same rule as `hot_sweep_setpoints`, up then down."""
    m2 = maximum**2
    for step in [0.01, 0.02, 0.05, 0.1, 0.2]:
        grid = [k * step for k in range(1, int(m2 / step + 1e-9) + 1) if k * step > 0.01 + 1e-9]
        if abs(grid[-1] - m2) > 1e-9:
            grid.append(m2)
        if len(grid) <= 20:
            break
    up = [0.1] + [np.sqrt(x) for x in grid]
    return up + up[-2::-1]


# R against I², from the 0.8 A run's up leg, to estimate each setpoint's power.
i2 = np.array([i**2 for i, _ in UP_LEG])
r = np.array([r * 1e-3 for _, r in UP_LEG])
fit = np.polyfit(i2, r, 2)

s = np.array(setpoints(MAXIMUM))
power = s**2 * np.polyval(fit, s**2)
for name, (k, tau) in FITS.items():
    for interval in [18, 21, 24]:
        t = np.arange(len(s)) * interval
        edges = np.concatenate([[t[0] - EDGE_SECONDS], (t[:-1] + t[1:]) / 2, [t[-1] + EDGE_SECONDS]])
        grid = np.arange(t[0] - EDGE_SECONDS, t[-1] + 1, 1.0)
        x = mount_resistance(grid, edges, power, k, tau)
        kelvin = {a: x.max() / (R0_OHMS * alpha) for a, alpha in ALPHA_PER_KELVIN.items()}
        print(
            f"{name} fit, {interval} s per point: peak {x.max() * 1e3:.2f} mΩ, "
            f"{kelvin['W-Re']:.1f} K (W {kelvin['W']:.1f} K), at {grid[x.argmax()]:.0f} s"
        )
```
