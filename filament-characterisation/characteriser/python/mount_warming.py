"""Estimates how much the mount warmed during a hot sweep.

Usage:

    .venv/bin/python mount_warming.py ../out/characterisation-<id>.json [...]

Unlike the other scripts here, the characteriser doesn't run this one: it's a
standalone analysis of a results file, printing a table for each.

At the same current, the filament settles to the same state within a second,
so the difference between a setpoint's resistance on the way down and on the
way up comes from something slower that changed between the two visits,
assumed to be the mount. The difference alone only gives the warming between
the visits, not since the start of the run, which is mostly larger: near the
peak, the mount had already warmed before the up visit. So this fits a model
of the mount's response to the filament's power to those differences, and uses
it to estimate the warming since the start at every point.
"""

import json
import sys
from pathlib import Path
from typing import Any

import numpy as np
from scipy.optimize import least_squares

# The filament's resistance with no current and the mount at room temperature,
# extrapolated from the hot sweeps. It only sets the scale of the conversion to
# kelvin, so its uncertainty matters much less than α's.
R0_OHMS = 0.0977

# Resistance's temperature coefficient near room temperature: W-Re's, the best
# estimate, and pure tungsten's, as a bound. They differ by about 45%, which
# makes α the largest uncertainty in the temperatures.
ALPHA_PER_KELVIN = {"W-Re": 0.0031, "W": 0.0045}

# How long before the first point, and after the last, the first and last
# points' power is taken to have been applied for. Points are about 20 s
# apart, so this is about half an interval.
EDGE_SECONDS = 10.0


def load(path: Path) -> list[dict[str, Any]]:
    """The hot sweep's points, in the order they were measured."""
    return json.loads(path.read_text())["hot_sweep"]["points"]


def mount_resistance(
    times: np.ndarray, edges: np.ndarray, power: np.ndarray, k: float, tau: float
) -> np.ndarray:
    """The extra filament resistance from the mount's warming, in ohms, at each
    of `times`.

    The mount is modelled as a first-order lag driven by the filament's power:

        dX/dt = (k P(t) - X) / tau

    where X is the extra resistance, k is its equilibrium value per watt and
    tau is the time constant. X is 0 at the start, i.e. the mount starts at
    room temperature.

    Point j's power, `power[j]`, is applied from `edges[j]` to
    `edges[j + 1]`. Over an interval of constant power the equation has an
    exact solution, and the equation is linear, so X is the sum of every
    interval's contribution: each one's rise while its power was on, decaying
    exponentially since.
    """
    result = np.zeros(len(times))

    for i, t in enumerate(times):
        for j, p in enumerate(power):
            start = edges[j]
            # Only the part of the interval before t has happened yet.
            end = min(edges[j + 1], t)
            if end <= start:
                continue

            # A step of power p from `start`, less the same step from `end`,
            # i.e. a pulse, evaluated at t.
            result[i] += k * p * (np.exp(-(t - end) / tau) - np.exp(-(t - start) / tau))

    return result


def analyse(path: Path) -> None:
    """Fits the model to one run and prints each setpoint's warming."""
    points = load(path)

    def values(field: str) -> np.ndarray:
        return np.array([p["analysis"][field]["value"] for p in points])

    # Each point's time is the middle of its sampling window.
    times = np.array([p["time_seconds"] for p in points])
    current = values("current_amps")
    resistance = values("resistance_ohms")
    u_resistance = np.array([p["analysis"]["resistance_ohms"]["uncertainty"] for p in points])
    power = current * values("voltage_volts")

    # Each point's power is applied from halfway after the previous point to
    # halfway before the next.
    edges = np.concatenate(
        [[times[0] - EDGE_SECONDS], (times[:-1] + times[1:]) / 2, [times[-1] + EDGE_SECONDS]]
    )

    # Pair each down point with the up point at the same setpoint. The peak is
    # only visited once, so it has no pair.
    up = {round(p["setpoint_amps"], 6): i for i, p in enumerate(points) if p["direction"] == "Up"}
    pairs = [
        (up[round(p["setpoint_amps"], 6)], j)
        for j, p in enumerate(points)
        if p["direction"] == "Down" and round(p["setpoint_amps"], 6) in up
    ]
    ups = np.array([a for a, _ in pairs])
    downs = np.array([b for _, b in pairs])

    # The measured warming between the visits. Taking the difference at the
    # same current cancels everything that depends only on current: the
    # filament's own heating, and the low-current voltage error.
    observed = resistance[downs] - resistance[ups]
    u_observed = np.hypot(u_resistance[ups], u_resistance[downs])

    # Each pair's misfit in standard uncertainties, so the fit weights the
    # pairs by how well they're known.
    def residuals(parameters: np.ndarray) -> np.ndarray:
        x = mount_resistance(times, edges, power, *parameters)
        return (x[downs] - x[ups] - observed) / u_observed

    # Started from the hold test's values in `docs/4_TEMPERATURE_COMPENSATION.md`,
    # about 0.1 Ω/W and 155 s. tau is kept above 5 s, much faster than any
    # warming seen, so it can't collapse to 0.
    fit = least_squares(residuals, [0.1, 155.0], bounds=([0.0, 5.0], [100.0, 1e6]))
    k, tau = fit.x

    # The parameters' standard uncertainties, scaled by the reduced chi-squared
    # so they reflect the actual scatter. k and tau are partly correlated: a
    # slower, stronger mount fits much like a faster, weaker one.
    dof = len(observed) - 2
    reduced_chi2 = np.sum(fit.fun**2) / dof
    covariance = np.linalg.inv(fit.jac.T @ fit.jac) * reduced_chi2
    u_k, u_tau = np.sqrt(np.diag(covariance))

    # Warming the whole filament by ΔT adds R₀ α ΔT to its resistance however
    # hot it already is (see "Why the same amount is subtracted at every current" in
    # `docs/4_TEMPERATURE_COMPENSATION.md`). Raising the clamp's temperature
    # lifts the whole wire's profile by about the same amount, so ΔT is the
    # temperature rise where the wire is clamped.
    x = mount_resistance(times, edges, power, k, tau)
    kelvin = {name: x / (R0_OHMS * alpha) for name, alpha in ALPHA_PER_KELVIN.items()}

    print(path.name)
    print(f"  k = {k:.3f} ± {u_k:.3f} mΩ/mW, tau = {tau:.0f} ± {u_tau:.0f} s")
    print(f"  reduced chi-squared {reduced_chi2:.1f} with {dof} degrees of freedom")
    for name, t in kelvin.items():
        print(
            f"  {name}: {k / (R0_OHMS * ALPHA_PER_KELVIN[name]):.0f} K/W, "
            f"peak {t.max():.1f} K at {times[t.argmax()]:.0f} s"
        )

    # The direct difference and the model's down − up should agree, since
    # that's what was fitted. The model's up and down columns are the warming
    # since the start, which the direct difference can't see.
    t = kelvin["W-Re"]
    print("  W-Re, in K:")
    print("  setpoint (mA)  direct  model up  model down  model down − up  ±")
    for (a, b), o, u in zip(pairs, observed, u_observed):
        scale = R0_OHMS * ALPHA_PER_KELVIN["W-Re"]
        print(
            f"  {points[a]['setpoint_amps'] * 1e3:13.0f}  {o / scale:6.1f}  {t[a]:8.1f}"
            f"  {t[b]:10.1f}  {t[b] - t[a]:15.1f}  {u / scale:.1f}"
        )
    print()


def main() -> int:
    if len(sys.argv) < 2:
        print(f"usage: {sys.argv[0]} RESULTS.json [...]", file=sys.stderr)
        return 1

    for path in sys.argv[1:]:
        analyse(Path(path))

    return 0


if __name__ == "__main__":
    sys.exit(main())
