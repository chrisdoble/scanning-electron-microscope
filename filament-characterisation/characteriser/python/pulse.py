"""Analyses a pulse run: the pulse's energy, the baseline, and how much the
pulse warmed the mount.

Reads a `PulseAnalysisInput` on stdin and prints a `PulseAnalysis` on stdout
(see `schemas/`). See "Analysis" in
`filament-characterisation/docs/6_PULSE_HEATING.md` for the method.
"""

import math
import sys
from dataclasses import dataclass
from typing import Any

import numpy as np
from scipy.optimize import least_squares
from uncertainties import correlated_values, ufloat

from analysis import AnalysisError, derived
from script_io import read_input, write_output

# The largest clamp rise, with the W-Re α, that a pulse should be planned to
# give (see "Using it safely" in the design document). Only used to work out
# the energy that would reach it; nothing enforces it.
CLAMP_RISE_BUDGET_KELVIN = 25.0

# A cool-down fit whose reduced chi-squared is above this is warned about: one
# time constant may not describe the cooling.
CHI_SQUARED_WARNING = 2.0


@dataclass
class Resistances:
    """Probe resistances from neighbouring pairs of blocks, with their times
    and covariance matrix."""

    t: np.ndarray
    r: np.ndarray
    covariance: np.ndarray


def probe_resistances(samples: list[dict[str, Any]]) -> Resistances:
    """Combines probe samples into resistances by polarity reversal, as the
    binary's `probe_resistances` does.

    Each block of consecutive samples in one polarity is averaged to one V, I
    and t. Every neighbouring pair of blocks in opposite polarities, a and b,
    gives R = |Vₐ − V_b| / (Iₐ + I_b), timed at the midpoint of the blocks'
    times. A magnitude, because which polarity gives the positive voltage
    depends on how the relays and sense leads are wired.

    Each block's mean voltage is uncertain by its standard error. Neighbouring
    pairs share a block, so their resistances are correlated, about 0.5 when
    the blocks are equally noisy. The covariance matrix tracks that, so fits
    don't treat them as independent. The currents' uncertainties are left out:
    the supply's readback is steadier, relatively, than the voltage by more
    than an order of magnitude.

    Blocks with fewer than two samples have no standard error, so they're
    dropped.
    """
    blocks = []
    for sample in samples:
        if blocks and blocks[-1]["polarity"] == sample["polarity"]:
            blocks[-1]["samples"].append(sample)
        else:
            blocks.append({"polarity": sample["polarity"], "samples": [sample]})
    blocks = [b for b in blocks if len(b["samples"]) >= 2]

    v = np.array([np.mean([s["voltage_volts"] for s in b["samples"]]) for b in blocks])
    i = np.array([np.mean([s["current_amps"] for s in b["samples"]]) for b in blocks])
    t = np.array([np.mean([s["time_seconds"] for s in b["samples"]]) for b in blocks])
    v_variance = np.array(
        [np.var([s["voltage_volts"] for s in b["samples"]], ddof=1) / len(b["samples"]) for b in blocks]
    )

    reversing = {("Forward", "Reverse"), ("Reverse", "Forward")}
    pairs = [
        k
        for k in range(len(blocks) - 1)
        if (blocks[k]["polarity"], blocks[k + 1]["polarity"]) in reversing
    ]

    # Each resistance is linear in the block voltages, so its covariance is
    # J diag(var V) Jᵀ, where J holds each resistance's derivative with
    # respect to each block's voltage.
    r = np.empty(len(pairs))
    jacobian = np.zeros((len(pairs), len(blocks)))
    for row, k in enumerate(pairs):
        difference = v[k] - v[k + 1]
        sign = 1.0 if difference >= 0 else -1.0
        total_current = i[k] + i[k + 1]
        r[row] = sign * difference / total_current
        jacobian[row, k] = sign / total_current
        jacobian[row, k + 1] = -sign / total_current

    return Resistances(
        t=np.array([(t[k] + t[k + 1]) / 2 for k in pairs]),
        r=r,
        covariance=jacobian @ np.diag(v_variance) @ jacobian.T,
    )


def fit(
    model: Any, t: np.ndarray, r: np.ndarray, covariance: np.ndarray, p0: list[float], bounds: Any
) -> tuple[list[Any], float, int]:
    """Fits `model` to `r` against `t`, returning the parameters, correlated,
    with the reduced chi-squared and degrees of freedom.

    Each point is weighted by its own variance only, ignoring the correlation
    between neighbouring pairs. A fit that used the full covariance
    (generalised least squares) would be badly conditioned: with neighbours
    correlated at about 0.5, the covariance is nearly singular for a pattern
    that alternates from pair to pair, which is exactly what a drifting offset
    leaves, so it would weight that pattern enormously instead of averaging it
    out.

    The correlation still goes into the uncertainties, with the "sandwich"
    covariance of a weighted least-squares estimate, (JᵀWJ)⁻¹ JᵀWCWJ (JᵀWJ)⁻¹,
    where C is the data's covariance and W the weights. See
    <https://en.wikipedia.org/wiki/Heteroskedasticity-consistent_standard_errors>.
    They're scaled up by the reduced chi-squared when that's above 1, so a
    misfit widens them rather than being ignored. The chi-squared is the
    weighted one, which correlation between neighbours leaves at about the
    degrees of freedom when the model fits.
    """
    sigma = np.sqrt(np.diag(covariance))

    def weighted_residuals(parameters: np.ndarray) -> np.ndarray:
        return (model(t, *parameters) - r) / sigma

    try:
        result = least_squares(weighted_residuals, p0, bounds=bounds)
    except (ValueError, np.linalg.LinAlgError) as e:
        raise AnalysisError(f"the fit failed: {e}") from e
    if not result.success:
        raise AnalysisError(f"the fit failed: {result.message}")

    # The Jacobian of the weighted residuals is W^½ J, so JᵀWJ is jᵀj and
    # JᵀWCWJ is jᵀ (W^½ C W^½) j, where W^½ C W^½ is the data's correlation
    # matrix.
    j = result.jac
    correlation = covariance / np.outer(sigma, sigma)
    try:
        bread = np.linalg.inv(j.T @ j)
    except np.linalg.LinAlgError as e:
        raise AnalysisError(f"the fit's parameters aren't determined: {e}") from e
    parameter_covariance = bread @ (j.T @ correlation @ j) @ bread

    dof = len(r) - len(result.x)
    reduced = float(np.sum(result.fun**2)) / dof if dof > 0 else math.nan
    if reduced > 1:
        parameter_covariance = parameter_covariance * reduced

    return correlated_values(result.x, parameter_covariance), reduced, dof


def analyse(data: dict[str, Any]) -> dict[str, Any]:
    """Analyses a `PulseAnalysisInput`, returning a `PulseAnalysis`.

    Raises `AnalysisError` if the run can't be analysed.
    """
    warnings = []
    up = data["pulse_up_seconds"]
    down = data["pulse_down_seconds"]

    # The pulse. The power is a magnitude, for the same reason as the
    # resistances.
    pulse_samples = data["pulse_samples"]
    if not pulse_samples:
        raise AnalysisError("the pulse has no samples")
    times = np.array([s["time_seconds"] for s in pulse_samples])
    voltage = np.abs([s["voltage_volts"] for s in pulse_samples])
    current = np.array([s["current_amps"] for s in pulse_samples])
    power = voltage * current

    # The time each sample's power is taken to cover: since the previous
    # sample, or since the current was set for the first. The last sample's
    # also covers the tail until the current was set back down. The binary
    # works out the energy the same way.
    durations = np.diff(np.concatenate([[up], times]))
    durations[-1] += down - times[-1]
    energy = float(np.sum(power * durations))
    duration = down - up
    hot_resistances = voltage / current

    # The baseline: a straight line, for its level and slope.
    baseline = probe_resistances(data["baseline"])
    if len(baseline.r) < 3:
        raise AnalysisError(
            f"the baseline has {len(baseline.r)} probe resistances; at least 3 are needed"
        )
    t_mean = float(np.mean(baseline.t))
    (level, slope), _, _ = fit(
        lambda t, a, b: a + b * (t - t_mean),
        baseline.t,
        baseline.r,
        baseline.covariance,
        [float(np.mean(baseline.r)), 0.0],
        (-np.inf, np.inf),
    )

    # The cool-down, leaving out the samples taken while the filament itself
    # cools, before pairing them.
    start = down + data["fit_skip_seconds"]
    cool_down = probe_resistances([s for s in data["cool_down"] if s["time_seconds"] >= start])
    if len(cool_down.r) < 4:
        raise AnalysisError(
            f"the cool-down has {len(cool_down.r)} probe resistances after the skip; at least 4 "
            "are needed"
        )

    # R(t) = R_b + A·e^(−t/τ), with t from the end of the pulse, fitted to the
    # baseline and the cool-down together so they share R_b, and its
    # uncertainty carries into A. The two sets' noise is independent.
    t = np.concatenate([baseline.t, cool_down.t])
    r = np.concatenate([baseline.r, cool_down.r])
    covariance = np.block(
        [
            [baseline.covariance, np.zeros((len(baseline.r), len(cool_down.r)))],
            [np.zeros((len(cool_down.r), len(baseline.r))), cool_down.covariance],
        ]
    )

    def model(t: np.ndarray, r_b: float, a: float, tau: float) -> np.ndarray:
        # The baseline is before the pulse, where nothing has warmed yet.
        return np.where(t >= down, r_b + a * np.exp(-(t - down) / tau), r_b)

    first_extra = float(cool_down.r[0] - level.nominal_value)
    (r_b, extra_at_end, tau), reduced, dof = fit(
        model,
        t,
        r,
        covariance,
        [level.nominal_value, max(first_extra, 1e-6), 120.0],
        ([-np.inf, -np.inf, 1.0], [np.inf, np.inf, 1e5]),
    )
    if reduced > CHI_SQUARED_WARNING:
        warnings.append(
            f"the cool-down fit's reduced chi-squared is {reduced:.1f}; one time constant may not "
            "describe the cooling"
        )

    # The first reading against the baseline's level: model-free, and a lower
    # bound on the warming at its time.
    first = ufloat(cool_down.r[0], math.sqrt(cool_down.covariance[0, 0])) - level

    # Kelvin from extra resistance with each α, as in
    # `MOUNT_WARMING_ANALYSIS.md`.
    r0 = data["cold_resistance_ohms"]
    alphas = data["temperature_coefficients_per_kelvin"]
    rise = {name: extra_at_end / (r0 * alpha) for name, alpha in alphas.items()}
    per_joule = {name: value / energy for name, value in rise.items()}

    budget_energy = None
    if extra_at_end.nominal_value > 0:
        budget_energy = derived(CLAMP_RISE_BUDGET_KELVIN / per_joule["tungsten_rhenium"])
    else:
        warnings.append("the fitted warming at the end of the pulse isn't positive")

    return {
        "baseline": {
            "resistance_ohms": derived(level),
            "slope_ohms_per_second": derived(slope),
        },
        "budget_energy_joules": budget_energy,
        "clamp_rise_budget_kelvin": CLAMP_RISE_BUDGET_KELVIN,
        "clamp_rise_kelvin": {name: derived(value) for name, value in rise.items()},
        "fit": {
            "baseline_resistance_ohms": derived(r_b),
            "degrees_of_freedom": dof,
            "extra_resistance_at_end_ohms": derived(extra_at_end),
            "reduced_chi_squared": reduced,
            "time_constant_seconds": derived(tau),
        },
        "first_reading": {
            "extra_resistance_ohms": derived(first),
            "seconds_after_pulse": float(cool_down.t[0] - down),
        },
        "kelvin_per_joule": {name: derived(value) for name, value in per_joule.items()},
        "pulse": {
            "duration_seconds": duration,
            "energy_joules": energy,
            "final_resistance_ohms": float(hot_resistances[-1]),
            "mean_power_watts": energy / duration,
            "peak_power_watts": float(np.max(power)),
            "resistances_ohms": hot_resistances.tolist(),
        },
        "warnings": warnings,
    }


def main() -> int:
    data = read_input("pulse")

    try:
        analysis = analyse(data)
    except AnalysisError as e:
        print(e, file=sys.stderr)
        return 1

    write_output(analysis)
    return 0


if __name__ == "__main__":
    sys.exit(main())
