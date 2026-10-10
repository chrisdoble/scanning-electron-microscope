"""Fits the cold-resistance setpoints and works out the uncertainty budget.

Reads a `ColdResistanceFitInput` on stdin and prints a `ColdResistanceAnalysis`
on stdout (see `schemas/`). See section 3.4 of
`filament-characterisation/docs/2_COLD_RESISTANCE.md` for the method, and
`filament-characterisation/docs/4_TEMPERATURE_COMPENSATION.md` for the
correction for the mount's warming.
"""

import math
import sys
from dataclasses import dataclass
from typing import Any

import numpy as np
from scipy.optimize import curve_fit
from scipy.stats import chi2
from uncertainties import UFloat, covariance_matrix, ufloat

from script_io import read_input, write_output


class AnalysisError(Exception):
    """The points can't be fitted. The message says why."""


@dataclass
class Points:
    """The fit's data: each point's $x = I^2$ with its standard uncertainty,
    and its resistance corrected for the mount's drift, with the corrected
    resistances' covariance matrix."""

    x: np.ndarray
    u_x: np.ndarray
    r: np.ndarray
    covariance: np.ndarray

    # The corrected resistances and the drift subtracted from each, with every
    # correlation tracked, in the order of the points.
    corrected: list[UFloat]
    drift: list[UFloat]

    # Each point's interpolation uncertainty, $u_{interp,i}$, which is part of
    # `drift`.
    u_interp: np.ndarray

    # The last reference's resistance less the first's.
    reference_drift: UFloat


@dataclass
class Fit:
    """A weighted straight-line fit, $R = R_0 + b x$."""

    r0: float
    b: float
    u_r0: float
    u_b: float

    chi_squared: float
    degrees_of_freedom: int


def measured(point: dict[str, Any], offset: float) -> tuple[UFloat, UFloat]:
    """A point's or reference's resistance and current, with the current
    shifted by `offset`, propagating uncertainties as the per-point analysis
    does."""
    v = ufloat(point["voltage_volts"]["value"], point["voltage_volts"]["uncertainty"])
    i = ufloat(point["current_amps"]["value"] - offset, point["current_amps"]["uncertainty"])
    return v / i, i


def interval_curvatures(times: np.ndarray, resistances: np.ndarray) -> np.ndarray:
    """An estimate of the drift's largest $|f''|$ in each interval between
    consecutive references: the larger of the second divided differences
    centred on the interval's two ends. With two references there are none,
    so the curvature is taken as 0.

    The last reference has no estimate, but the drift has levelled off by
    then, so the last interval's single estimate is enough. The first
    reference has none either, and that's where the drift curves most, so its
    estimate is extrapolated back from the next two, assuming the curvature
    shrinks geometrically, as an exponential approach's does. It's never less
    than the next one's, and reference noise can only make it larger."""
    centred = np.zeros(len(times))
    for k in range(1, len(times) - 1):
        before = (resistances[k] - resistances[k - 1]) / (times[k] - times[k - 1])
        after = (resistances[k + 1] - resistances[k]) / (times[k + 1] - times[k])
        centred[k] = abs(2 * (after - before) / (times[k + 1] - times[k - 1]))

    if len(times) >= 4 and centred[2] > 0:
        ratio = centred[1] / centred[2]
        exponent = (times[1] - times[0]) / (times[2] - times[1])
        centred[0] = max(centred[1], centred[1] * ratio**exponent)

    return np.maximum(centred[:-1], centred[1:])


def to_points(
    points: list[dict[str, Any]], references: list[dict[str, Any]], offset: float
) -> Points:
    """Each point's $x$, and its $R$ less the mount's drift since the first
    reference, interpolated linearly between the references either side of it.

    `offset` is a hypothetical readback offset $\\delta$, subtracted from every
    point's and reference's current before anything else is computed. It's 0
    for the nominal fit, and $\\pm\\delta_{max}$ for the corner analysis in
    `offset_corners`.

    Raises `AnalysisError` if there are fewer than two references, their times
    don't increase, or a point isn't between the first and last.
    """
    if len(references) < 2:
        raise AnalysisError(f"expected at least 2 references, got {len(references)}")
    times = np.array([reference["time_seconds"] for reference in references])
    if np.any(np.diff(times) <= 0):
        raise AnalysisError(f"the references' times must increase: {times}")

    # Every input is created once, so the covariance matrix sees which
    # references the points share.
    reference_resistances = [measured(reference, offset)[0] for reference in references]
    curvatures = interval_curvatures(times, np.array([r.nominal_value for r in reference_resistances]))

    x, u_x, corrected, drift, u_interp = [], [], [], [], []
    for point in points:
        t = point["time_seconds"]
        if not times[0] <= t <= times[-1]:
            raise AnalysisError(
                f"a point at {t:.1f} s isn't between the first and last references, "
                f"at {times[0]:.1f} s and {times[-1]:.1f} s"
            )

        # The interval the point is in, and how far through it.
        j = min(int(np.searchsorted(times, t, side="right")) - 1, len(times) - 2)
        w = (t - times[j]) / (times[j + 1] - times[j])

        point_drift = (
            (1 - w) * reference_resistances[j]
            + w * reference_resistances[j + 1]
            - reference_resistances[0]
        )

        # The bound on linear interpolation's error, treated as rectangular
        # and independent between points. It's 0 without curvature, which
        # `ufloat` warns about.
        bound = curvatures[j] * (t - times[j]) * (times[j + 1] - t) / 2
        interpolation = bound / math.sqrt(3)
        if interpolation > 0:
            point_drift += ufloat(0.0, interpolation)
        resistance, i = measured(point, offset)
        squared = i**2

        x.append(squared.nominal_value)
        u_x.append(squared.std_dev)
        corrected.append(resistance - point_drift)
        drift.append(point_drift)
        u_interp.append(interpolation)

    return Points(
        x=np.array(x),
        u_x=np.array(u_x),
        r=np.array([r.nominal_value for r in corrected]),
        covariance=np.array(covariance_matrix(corrected)),
        corrected=corrected,
        drift=drift,
        u_interp=np.array(u_interp),
        reference_drift=reference_resistances[-1] - reference_resistances[0],
    )


def line(x: np.ndarray, r0: float, b: float) -> np.ndarray:
    return r0 + b * x


def fit_line(x: np.ndarray, r: np.ndarray, covariance: np.ndarray) -> Fit:
    """Fits $R = R_0 + b x$ by generalised least squares, given the
    resistances' covariance matrix.

    Raises `AnalysisError` if there are too few points for a goodness of fit,
    or the covariance matrix isn't finite, symmetric and positive definite.
    """
    # Two parameters, so at least one degree of freedom is left for chi-squared.
    if len(x) < 3:
        raise AnalysisError(f"expected at least 3 points, got {len(x)}")

    # Otherwise the fit can't invert it: a point with zero variance would have
    # infinite weight.
    if not np.all(np.isfinite(covariance)) or not np.allclose(covariance, covariance.T):
        raise AnalysisError("the resistances' covariance matrix must be finite and symmetric")
    try:
        np.linalg.cholesky(covariance)
    except np.linalg.LinAlgError:
        raise AnalysisError("the resistances' covariance matrix must be positive definite") from None

    # A 2-D `sigma` is the data's covariance matrix, and `curve_fit` whitens
    # the residuals with it. `absolute_sigma=True` stops it scaling the
    # parameters' covariance by the reduced chi-squared, which the Birge
    # inflation does deliberately instead (and never as a deflation).
    popt, pcov, info, _, _ = curve_fit(
        line,
        x,
        r,
        sigma=covariance,
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
        # `fvec` is the whitened residuals.
        chi_squared=float(np.sum(info["fvec"] ** 2)),
        degrees_of_freedom=len(x) - 2,
    )


def offset_corners(
    points: list[dict[str, Any]], references: list[dict[str, Any]], offset_bound: float
) -> tuple[float, float]:
    """The fitted $R_0$ with the readback offset at $-\\delta_{max}$ and
    $+\\delta_{max}$, in that order. The order doesn't matter to the
    uncertainty, which only uses the difference; it's fixed so the stored
    `offset_corner_resistances_ohms` says which is which.

    The offset doesn't reverse with the current and isn't a scale factor on
    $R_0$, so it's found by refitting with the currents shifted. The
    references' currents are shifted too, and the drift corrected afresh.
    $R_0$ is very nearly linear in the offset, so the extremes are at the two
    ends.
    """
    corners = []
    for offset in (-offset_bound, offset_bound):
        shifted = to_points(points, references, offset)
        corners.append(fit_line(shifted.x, shifted.r, shifted.covariance).r0)
    return corners[0], corners[1]


def derived(quantity: UFloat) -> dict[str, float]:
    """`quantity` as a `Derived`."""
    return {"value": quantity.nominal_value, "uncertainty": quantity.std_dev}


def analyse(data: dict[str, Any]) -> dict[str, Any]:
    """Analyses a `ColdResistanceFitInput`, returning a
    `ColdResistanceAnalysis`.

    Raises `AnalysisError` if the points can't be fitted.
    """
    parameters = data["parameters"]
    points = data["points"]
    references = data["references"]
    warnings = []

    nominal = to_points(points, references, 0.0)
    fit = fit_line(nominal.x, nominal.r, nominal.covariance)
    r0 = fit.r0

    # The uncertainty in x is ignored by the fit. Say so if it isn't small.
    x_ratio = np.max(abs(fit.b) * nominal.u_x / np.sqrt(np.diag(nominal.covariance)))
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
    corners = offset_corners(points, references, parameters["current_offset_bound_amps"])
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
        "corrected_resistances_ohms": [derived(r) for r in nominal.corrected],
        "current_gain_uncertainty_ohms": current_gain_uncertainty,
        "current_offset_uncertainty_ohms": current_offset_uncertainty,
        "drift_corrections_ohms": [derived(d) for d in nominal.drift],
        "fit_uncertainty_ohms": fit_uncertainty,
        "offset_corner_resistances_ohms": list(corners),
        "reduced_chi_squared": reduced_chi_squared,
        "reference_drift_ohms": derived(nominal.reference_drift),
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
