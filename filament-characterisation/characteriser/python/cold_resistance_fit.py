"""Fits the cold-resistance setpoints and works out the uncertainty budget.

Reads a `ColdResistanceFitInput` on stdin and prints a `ColdResistanceAnalysis`
on stdout (see `schemas/`). See section 3.4 of
`filament-characterisation/docs/2_COLD_RESISTANCE.md` for the method.
"""

import math
import sys
from dataclasses import dataclass
from typing import Any

import numpy as np
from scipy.optimize import curve_fit
from scipy.stats import chi2
from uncertainties import ufloat

from script_io import read_input, write_output


class AnalysisError(Exception):
    """The points can't be fitted. The message says why."""


@dataclass
class Points:
    """The fit's data: each point's $x = I^2$ and resistance, with their
    standard uncertainties."""

    x: np.ndarray
    u_x: np.ndarray
    r: np.ndarray
    u_r: np.ndarray


@dataclass
class Fit:
    """A weighted straight-line fit, $R = R_0 + b x$."""

    r0: float
    b: float
    u_r0: float
    u_b: float

    chi_squared: float
    degrees_of_freedom: int


def to_points(points: list[dict[str, Any]], offset: float) -> Points:
    """Each point's $x$ and $R$, propagating uncertainties as the per-point
    analysis does.

    `offset` is a hypothetical readback offset $\\delta$, subtracted from every
    current before $x$ and $R$ are computed. It's 0 for the nominal fit, and
    $\\pm\\delta_{max}$ for the corner analysis in `offset_corners`.
    """
    x, u_x, r, u_r = [], [], [], []
    for point in points:
        v = ufloat(point["voltage_volts"]["value"], point["voltage_volts"]["uncertainty"])
        i = ufloat(
            point["current_amps"]["value"] - offset,
            point["current_amps"]["uncertainty"],
        )
        resistance = v / i
        squared = i**2
        x.append(squared.nominal_value)
        u_x.append(squared.std_dev)
        r.append(resistance.nominal_value)
        u_r.append(resistance.std_dev)
    return Points(np.array(x), np.array(u_x), np.array(r), np.array(u_r))


def line(x: np.ndarray, r0: float, b: float) -> np.ndarray:
    return r0 + b * x


def fit_line(x: np.ndarray, r: np.ndarray, u_r: np.ndarray) -> Fit:
    """Fits $R = R_0 + b x$, weighting each point by $1/u_r^2$.

    Raises `AnalysisError` if there are too few points for a goodness of fit,
    or an uncertainty isn't finite and positive.
    """
    # Two parameters, so at least one degree of freedom is left for chi-squared.
    if len(x) < 3:
        raise AnalysisError(f"expected at least 3 points, got {len(x)}")

    # A zero would make its weight infinite.
    if not np.all(np.isfinite(u_r) & (u_r > 0)):
        raise AnalysisError(f"every resistance uncertainty must be finite and positive: {u_r}")

    # `sigma` is the standard uncertainties, not weights: `curve_fit` applies
    # the 1/u^2 weighting itself. `absolute_sigma=True` stops it scaling the
    # covariance by the reduced chi-squared, which the Birge inflation does
    # deliberately instead (and never as a deflation).
    popt, pcov, info, _, _ = curve_fit(
        line,
        x,
        r,
        sigma=u_r,
        absolute_sigma=True,
        p0=[r[0], 0.0],
        full_output=True,
    )
    r0, b = popt
    u_r0, u_b = np.sqrt(np.diag(pcov))

    return Fit(
        r0=float(r0),
        b=float(b),
        u_r0=float(u_r0),
        u_b=float(u_b),
        # `fvec` is the weighted residuals.
        chi_squared=float(np.sum(info["fvec"] ** 2)),
        degrees_of_freedom=len(x) - 2,
    )


def offset_corners(points: list[dict[str, Any]], offset_bound: float) -> tuple[float, float]:
    """The fitted $R_0$ with the readback offset at $-\\delta_{max}$ and
    $+\\delta_{max}$, in that order. The order doesn't matter to the
    uncertainty, which only uses the difference; it's fixed so the stored
    `offset_corner_resistances_ohms` says which is which.

    The offset doesn't reverse with the current and isn't a scale factor on
    $R_0$, so it's found by refitting with the currents shifted. $R_0$ is very
    nearly linear in the offset, so the extremes are at the two ends.
    """
    corners = []
    for offset in (-offset_bound, offset_bound):
        shifted = to_points(points, offset)
        corners.append(fit_line(shifted.x, shifted.r, shifted.u_r).r0)
    return corners[0], corners[1]


def analyse(data: dict[str, Any]) -> dict[str, Any]:
    """Analyses a `ColdResistanceFitInput`, returning a
    `ColdResistanceAnalysis`.

    Raises `AnalysisError` if the points can't be fitted.
    """
    parameters = data["parameters"]
    points = data["points"]
    warnings = []

    nominal = to_points(points, 0.0)
    fit = fit_line(nominal.x, nominal.r, nominal.u_r)
    r0 = fit.r0

    # The uncertainty in x is ignored by the fit. Say so if it isn't small.
    x_ratio = np.max(abs(fit.b) * nominal.u_x / nominal.u_r)
    if x_ratio > 0.1:
        warnings.append(
            f"the uncertainty in I² isn't negligible: it's up to {x_ratio:.2f} "
            f"of a point's resistance uncertainty"
        )

    # Goodness of fit.
    reduced_chi_squared = fit.chi_squared / fit.degrees_of_freedom
    p_value = float(chi2.sf(fit.chi_squared, fit.degrees_of_freedom))
    if p_value < 0.025:
        warnings.append(
            f"the points scatter more than their uncertainties explain "
            f"(p = {p_value:.3g}): the uncertainties may be underestimated, "
            f"points may not have settled, or the line may not fit"
        )
    elif p_value > 0.975:
        warnings.append(
            f"the points scatter less than their uncertainties predict "
            f"(p = {p_value:.3g}): the uncertainties may be overestimated, or "
            f"the points correlated"
        )

    # The Birge ratio inflates the statistical uncertainty to cover scatter
    # the per-point uncertainties missed, and is never applied as a deflation.
    # It's equivalent to inflating every point's uncertainty by a common
    # factor, which scales both parameters' uncertainties, so the slope gets it
    # too.
    birge_ratio = math.sqrt(max(1.0, reduced_chi_squared))
    fit_uncertainty = fit.u_r0 * birge_ratio
    slope_uncertainty = fit.u_b * birge_ratio

    # Systematic terms, each treated as rectangular.
    voltage_gain_uncertainty = r0 * parameters["voltage_gain_bound"] / math.sqrt(3)
    current_gain_uncertainty = r0 * parameters["current_gain_bound"] / math.sqrt(3)
    corners = offset_corners(points, parameters["current_offset_bound_amps"])
    current_offset_uncertainty = abs(corners[1] - corners[0]) / 2 / math.sqrt(3)

    combined_uncertainty = math.sqrt(
        fit_uncertainty**2
        + voltage_gain_uncertainty**2
        + current_gain_uncertainty**2
        + current_offset_uncertainty**2
    )

    # Correction to the reference temperature.
    alpha = parameters["temperature_coefficient_per_kelvin"]
    delta_t = parameters["filament_temperature_celsius"] - parameters["reference_temperature_celsius"]
    f = 1 / (1 + alpha * delta_t)
    reference_resistance = r0 * f
    u_temperature = parameters["filament_temperature_bound_kelvin"] / math.sqrt(3)
    u_alpha = parameters["temperature_coefficient_bound_per_kelvin"] / math.sqrt(3)
    temperature_uncertainty = reference_resistance * alpha * f * u_temperature
    temperature_coefficient_uncertainty = reference_resistance * abs(delta_t) * f * u_alpha
    reference_combined_uncertainty = math.sqrt(
        (f * combined_uncertainty) ** 2
        + temperature_uncertainty**2
        + temperature_coefficient_uncertainty**2
    )

    return {
        "chi_squared_p_value": p_value,
        "current_gain_uncertainty_ohms": current_gain_uncertainty,
        "current_offset_uncertainty_ohms": current_offset_uncertainty,
        "fit_uncertainty_ohms": fit_uncertainty,
        "offset_corner_resistances_ohms": list(corners),
        "reduced_chi_squared": reduced_chi_squared,
        "reference_resistance_ohms": {
            "value": reference_resistance,
            "uncertainty": reference_combined_uncertainty,
        },
        "resistance_ohms": {"value": r0, "uncertainty": combined_uncertainty},
        "slope_ohms_per_amp_squared": {"value": fit.b, "uncertainty": slope_uncertainty},
        "temperature_coefficient_uncertainty_ohms": temperature_coefficient_uncertainty,
        "temperature_uncertainty_ohms": temperature_uncertainty,
        "voltage_gain_uncertainty_ohms": voltage_gain_uncertainty,
        "warnings": warnings,
    }


def main() -> int:
    data = read_input("cold_resistance_fit")

    try:
        analysis = analyse(data)
    except AnalysisError as e:
        print(e, file=sys.stderr)
        return 1

    write_output(analysis)
    return 0


if __name__ == "__main__":
    sys.exit(main())
