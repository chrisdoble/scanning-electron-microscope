"""Tests for the cold-resistance analysis scripts.

Run from this directory with `python -m unittest`. The cases are the ones in
the "Tests" sections of `filament-characterisation/docs/2_COLD_RESISTANCE.md`
and `filament-characterisation/docs/4_TEMPERATURE_COMPENSATION.md`.
"""

import itertools
import json
import math
import subprocess
import sys
import unittest
import warnings
from pathlib import Path
from typing import Any, Callable

import jsonschema
import numpy as np

import cold_resistance_fit
import cold_resistance_point

PYTHON_DIR = Path(__file__).parent

# The fixture: the default setpoints, and noiseless voltages from
# R(I) = R0 + b I^2, where b makes the resistance 2% above R0 at 300 mA.
R0 = 0.096
B = 0.096 * 0.02 / 0.09
SETPOINTS = [0.100 + 0.025 * k for k in range(9)]
VOLTAGE_UNCERTAINTY = 1e-6
CURRENT_UNCERTAINTY = 20e-6

# The references' current, and how long each point or reference takes to
# measure, so the fixture's times follow a run's.
REFERENCE_CURRENT = 0.2
MEASUREMENT_SECONDS = 40.0

PARAMETERS = {
    "current_gain_bound": 0.0015,
    "current_offset_bound_amps": 0.005,
    "filament_temperature_bound_kelvin": 1.0,
    "filament_temperature_celsius": 25.0,
    "reference_temperature_celsius": 20.0,
    "temperature_coefficient_bound_per_kelvin": 0.0003,
    "temperature_coefficient_per_kelvin": 0.0045,
    "voltage_gain_bound": 0.02,
}


def schedule() -> tuple[list[float], list[float]]:
    """The times of the fixture's points and references, in the order the
    characteriser's `measurement_plan` measures them: a reference first, after
    the first point, after every second point after that, and last."""
    clock = (MEASUREMENT_SECONDS * k for k in itertools.count())
    reference_times = [next(clock)]
    point_times = []
    for index in range(len(SETPOINTS)):
        point_times.append(next(clock))
        if index % 2 == 0 or index == len(SETPOINTS) - 1:
            reference_times.append(next(clock))
    return point_times, reference_times


def fixture_input(
    drift: Callable[[float], float] = lambda t: 0.0,
    voltage_scale: float = 1.0,
    current_scale: float = 1.0,
    rng: np.random.Generator | None = None,
) -> dict[str, Any]:
    """The fixture as a `ColdResistanceFitInput`, with `drift(t)` added to
    every point's and reference's resistance, every voltage and current
    optionally scaled, and noise at their stated uncertainties if `rng` is
    given."""

    def entry(i: float, t: float) -> dict[str, Any]:
        v = (R0 + B * i**2 + drift(t)) * i * voltage_scale
        current = i * current_scale
        if rng is not None:
            v += rng.normal(0, VOLTAGE_UNCERTAINTY)
            current += rng.normal(0, CURRENT_UNCERTAINTY)
        return {
            "voltage_volts": {"value": v, "uncertainty": VOLTAGE_UNCERTAINTY},
            "current_amps": {"value": current, "uncertainty": CURRENT_UNCERTAINTY},
            "time_seconds": t,
        }

    point_times, reference_times = schedule()
    return {
        "parameters": PARAMETERS,
        "points": [entry(i, t) for i, t in zip(SETPOINTS, point_times)],
        "references": [entry(REFERENCE_CURRENT, t) for t in reference_times],
    }


def fixture_points(data: dict[str, Any], offset: float = 0.0) -> cold_resistance_fit.Points:
    return cold_resistance_fit.to_points(data["points"], data["references"], offset)


def fit_fixture(data: dict[str, Any]) -> cold_resistance_fit.Fit:
    points = fixture_points(data)
    return cold_resistance_fit.fit_line(points.x, points.r, points.covariance)


def uncorrected(data: dict[str, Any], offset: float = 0.0) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """Each point's $x$, $R$ and $u(R)$ without the drift correction."""
    measured = [cold_resistance_fit.measured(point, offset) for point in data["points"]]
    return (
        np.array([(i**2).nominal_value for _, i in measured]),
        np.array([r.nominal_value for r, _ in measured]),
        np.array([r.std_dev for r, _ in measured]),
    )


def measurement(value: float, uncertainty: float = 0.0) -> dict[str, Any]:
    return {"samples": [value, value], "value": value, "uncertainty": uncertainty}


def point_input(positive_voltage: float = 0.0096, negative_voltage: float = -0.0096) -> dict[str, Any]:
    """A `ColdResistancePointInput` at 100 mA."""
    return {
        "current_resolution_amps": 1e-4,
        "negative_current_amps": measurement(0.1),
        "negative_voltage_volts": measurement(negative_voltage, 1e-6),
        "offset_warning_volts": 0.006,
        "positive_current_amps": measurement(0.1),
        "positive_voltage_volts": measurement(positive_voltage, 1e-6),
    }


def run_script(script: str, data: Any) -> subprocess.CompletedProcess:
    """Runs `script` with `data` as JSON on stdin."""
    return subprocess.run(
        [sys.executable, str(PYTHON_DIR / script)],
        input=json.dumps(data),
        capture_output=True,
        text=True,
        cwd=PYTHON_DIR,
    )


def schema(name: str) -> dict[str, Any]:
    return json.loads((PYTHON_DIR / "schemas" / f"{name}.json").read_text())


class TestFit(unittest.TestCase):
    def test_recovery(self) -> None:
        fit = fit_fixture(fixture_input())
        self.assertAlmostEqual(fit.r0, R0, delta=R0 * 1e-9)
        self.assertAlmostEqual(fit.chi_squared, 0.0, delta=1e-12)

    def test_gain_invariance(self) -> None:
        epsilon = 0.01
        fit = fit_fixture(fixture_input(voltage_scale=1 + epsilon))
        self.assertAlmostEqual(fit.r0, R0 * (1 + epsilon), delta=R0 * 1e-9)

        g = 0.0015
        fit = fit_fixture(fixture_input(current_scale=1 + g))
        self.assertAlmostEqual(fit.r0, R0 / (1 + g), delta=R0 * 1e-9)

    def test_offset_corners(self) -> None:
        # The shifted points don't lie on a line, so the corners depend on the
        # weights, which include the references' uncertainties.
        data = fixture_input()
        lower, upper = cold_resistance_fit.offset_corners(data["points"], data["references"], 0.005)
        self.assertAlmostEqual(upper, 0.100195, delta=0.100195 * 1e-5)
        self.assertAlmostEqual(lower, 0.092013, delta=0.092013 * 1e-5)

        analysis = cold_resistance_fit.analyse(data)
        self.assertAlmostEqual(
            analysis["current_offset_uncertainty_ohms"], 0.002362, delta=0.002362 * 1e-4
        )

    def test_temperature_correction(self) -> None:
        analysis = cold_resistance_fit.analyse(fixture_input())
        self.assertAlmostEqual(
            analysis["reference_resistance_ohms"]["value"], 0.093888, delta=0.093888 * 1e-5
        )
        self.assertAlmostEqual(analysis["temperature_uncertainty_ohms"], 2.39e-4, delta=2.39e-4 * 5e-3)
        self.assertAlmostEqual(
            analysis["temperature_coefficient_uncertainty_ohms"], 7.95e-5, delta=7.95e-5 * 5e-3
        )

        at_reference = cold_resistance_fit.analyse(
            {
                **fixture_input(),
                "parameters": {**PARAMETERS, "filament_temperature_celsius": 20.0},
            }
        )
        self.assertEqual(
            at_reference["reference_resistance_ohms"]["value"],
            at_reference["resistance_ohms"]["value"],
        )
        self.assertEqual(at_reference["temperature_coefficient_uncertainty_ohms"], 0.0)

    def test_matches_closed_form(self) -> None:
        # Noisy, correlated data, fitted by curve_fit and by the closed-form
        # generalised least squares. A missing `absolute_sigma=True` would
        # scale the fit's uncertainties by the square root of the reduced
        # chi-squared.
        rng = np.random.default_rng(1)
        x = np.array(SETPOINTS) ** 2
        mixing = rng.normal(0, 10e-6, (len(x), len(x)))
        covariance = mixing @ mixing.T + np.diag(rng.uniform(5e-6, 30e-6, len(x)) ** 2)
        r = R0 + B * x + rng.multivariate_normal(np.zeros(len(x)), covariance)

        fit = cold_resistance_fit.fit_line(x, r, covariance)

        design = np.column_stack([np.ones(len(x)), x])
        inverse = np.linalg.inv(covariance)
        parameter_covariance = np.linalg.inv(design.T @ inverse @ design)
        r0, b = parameter_covariance @ design.T @ inverse @ r
        residuals = r - design @ np.array([r0, b])
        expected = {
            "r0": r0,
            "b": b,
            "u_r0": math.sqrt(parameter_covariance[0, 0]),
            "u_b": math.sqrt(parameter_covariance[1, 1]),
            "chi_squared": residuals @ inverse @ residuals,
        }
        for name, value in expected.items():
            with self.subTest(name):
                self.assertAlmostEqual(getattr(fit, name), value, delta=abs(value) * 1e-5)

    def test_rejects_a_covariance_matrix_that_isnt_positive_definite(self) -> None:
        x = np.array(SETPOINTS[:3]) ** 2
        with self.assertRaisesRegex(cold_resistance_fit.AnalysisError, "positive definite"):
            indefinite = np.array([[1.0, 2.0, 0.0], [2.0, 1.0, 0.0], [0.0, 0.0, 1.0]]) * 1e-10
            cold_resistance_fit.fit_line(x, R0 + B * x, indefinite)


class TestDriftCorrection(unittest.TestCase):
    def test_no_drift(self) -> None:
        data = fixture_input()
        analysis = cold_resistance_fit.analyse(data)
        x, r, u_r = uncorrected(data)

        corrected = np.array([d["value"] for d in analysis["corrected_resistances_ohms"]])
        np.testing.assert_allclose(corrected, r, rtol=1e-12)

        expected = cold_resistance_fit.fit_line(x, r, np.diag(u_r**2))
        self.assertAlmostEqual(analysis["resistance_ohms"]["value"], expected.r0, delta=expected.r0 * 1e-9)
        self.assertAlmostEqual(
            analysis["slope_ohms_per_amp_squared"]["value"], expected.b, delta=expected.b * 1e-9
        )

    def test_linear_drift_is_removed_exactly(self) -> None:
        # Linear interpolation is exact for a straight-line drift. 1 µΩ/s is
        # about 0.6 mΩ over the run.
        fit = fit_fixture(fixture_input(drift=lambda t: 1e-6 * t))
        self.assertAlmostEqual(fit.r0, R0, delta=R0 * 1e-9)
        self.assertAlmostEqual(fit.b, B, delta=B * 1e-6)
        self.assertAlmostEqual(fit.chi_squared, 0.0, delta=1e-9)

    def test_interpolation_bounds_cover_exponential_drift(self) -> None:
        # Each point's actual interpolation error is within its bound, for time
        # constants either side of the hold test's. The first interval's is
        # extrapolated, which this checks most.
        point_times, _ = schedule()
        for tau in (60, 95, 155, 250):

            def drift(t: float) -> float:
                return 400e-6 * (1 - math.exp(-t / tau))

            points = fixture_points(fixture_input(drift=drift))
            bounds = points.u_interp * math.sqrt(3)
            for t, d, bound in zip(point_times, points.drift, bounds):
                with self.subTest(tau=tau, t=t):
                    self.assertLessEqual(abs(d.nominal_value - drift(t)), bound)

    def test_exponential_drift(self) -> None:
        # Like the hold test: 400 µΩ with a 155 s time constant.
        def drift(t: float) -> float:
            return 400e-6 * (1 - math.exp(-t / 155))

        points = fixture_points(fixture_input(drift=drift))

        # R0 is recovered to within its statistical uncertainty, which
        # includes the interpolation's.
        fit = cold_resistance_fit.fit_line(points.x, points.r, points.covariance)
        self.assertLess(abs(fit.r0 - R0), fit.u_r0)

        # With noise added, the points scatter as their uncertainties say.
        analysis = cold_resistance_fit.analyse(fixture_input(drift=drift, rng=np.random.default_rng(2)))
        self.assertGreater(analysis["chi_squared_p_value"], 0.025)
        self.assertLess(analysis["chi_squared_p_value"], 0.975)

    def test_propagation(self) -> None:
        # Exact currents of 1 A, so each resistance's uncertainty is its
        # voltage's. The references don't drift, so there's no interpolation
        # uncertainty.
        def entry(resistance: float, u: float, t: float) -> dict[str, Any]:
            return {
                "voltage_volts": {"value": resistance, "uncertainty": u},
                "current_amps": {"value": 1.0, "uncertainty": 0.0},
                "time_seconds": t,
            }

        u_ref = [10e-6, 20e-6, 30e-6]
        references = [entry(R0, u, t) for u, t in zip(u_ref, [0.0, 100.0, 200.0])]
        u_point = [15e-6, 25e-6, 35e-6, 45e-6]
        weights = [0.5, 0.25, 0.25, 0.75]
        times = [50.0, 25.0, 125.0, 175.0]
        points = [entry(R0, u, t) for u, t in zip(u_point, times)]

        # `uncertainties` warns about the exact currents, which don't matter.
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", UserWarning)
            covariance = cold_resistance_fit.to_points(points, references, 0.0).covariance
        w = weights

        # A point midway between references 0 and 1.
        expected = u_point[0] ** 2 + u_ref[1] ** 2 / 4 + u_ref[0] ** 2 / 4
        self.assertAlmostEqual(covariance[0, 0], expected, delta=expected * 1e-12)

        # Two points between references 0 and 1.
        expected = w[0] * w[1] * (u_ref[0] ** 2 + u_ref[1] ** 2)
        self.assertAlmostEqual(covariance[0, 1], expected, delta=expected * 1e-12)

        # Two points between references 1 and 2.
        expected = (
            (1 - w[2]) * (1 - w[3]) * u_ref[1] ** 2 + w[2] * w[3] * u_ref[2] ** 2 + u_ref[0] ** 2
        )
        self.assertAlmostEqual(covariance[2, 3], expected, delta=expected * 1e-12)

    def test_corner_analysis_shifts_the_references(self) -> None:
        data = fixture_input(drift=lambda t: 1e-6 * t)
        point_times, reference_times = schedule()

        for offset in (-0.005, 0.005):
            with self.subTest(offset=offset):
                # The correction by hand, with every current shifted.
                x, r, _ = uncorrected(data, offset)
                references = np.array(
                    [
                        cold_resistance_fit.measured(reference, offset)[0].nominal_value
                        for reference in data["references"]
                    ]
                )
                drift = np.interp(point_times, reference_times, references) - references[0]
                points = fixture_points(data, offset)
                np.testing.assert_allclose(points.r, r - drift, rtol=1e-12)

        lower, upper = cold_resistance_fit.offset_corners(data["points"], data["references"], 0.005)
        for corner, offset in ((lower, -0.005), (upper, 0.005)):
            points = fixture_points(data, offset)
            fit = cold_resistance_fit.fit_line(points.x, points.r, points.covariance)
            self.assertEqual(corner, fit.r0)

    def test_errors(self) -> None:
        data = fixture_input()
        one_reference = {**data, "references": data["references"][:1]}
        outside = {**data, "points": [{**data["points"][0], "time_seconds": -1.0}] + data["points"][1:]}
        unordered = {**data, "references": data["references"][::-1]}

        for name, case, message in [
            ("one reference", one_reference, "at least 2 references"),
            ("outside", outside, "isn't between the first and last references"),
            ("unordered", unordered, "times must increase"),
        ]:
            with self.subTest(name):
                result = run_script("cold_resistance_fit.py", case)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(message, result.stderr)


class TestPoint(unittest.TestCase):
    def test_quantisation_floor(self) -> None:
        expected = 1e-4 / math.sqrt(12)
        self.assertAlmostEqual(
            cold_resistance_point.current_uncertainty(0.0, 1e-4), expected, delta=expected * 1e-12
        )

    def test_positive_polarity_must_be_positive(self) -> None:
        result = run_script("cold_resistance_point.py", point_input(positive_voltage=-0.001))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("positive polarity", result.stderr)

    def test_negative_polarity_must_be_negative(self) -> None:
        result = run_script("cold_resistance_point.py", point_input(negative_voltage=0.001))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("negative polarity", result.stderr)


class TestSchemas(unittest.TestCase):
    def assert_output_valid(self, script: str, data: Any) -> None:
        result = run_script(f"{script}.py", data)
        self.assertEqual(result.returncode, 0, result.stderr)
        jsonschema.validate(json.loads(result.stdout), schema(f"{script}.output"))

    def test_outputs_match_their_schemas(self) -> None:
        self.assert_output_valid("mean_and_standard_error", {"samples": [1.0, 2.0, 3.0]})
        self.assert_output_valid("cold_resistance_point", point_input())
        self.assert_output_valid("cold_resistance_fit", fixture_input())

    def test_invalid_input_is_rejected(self) -> None:
        missing = point_input()
        del missing["offset_warning_volts"]
        unknown = {**point_input(), "unexpected": 1}
        wrong_type = {**point_input(), "current_resolution_amps": "0.0001"}

        for name, data in [("missing", missing), ("unknown", unknown), ("wrong type", wrong_type)]:
            with self.subTest(name):
                result = run_script("cold_resistance_point.py", data)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("invalid input", result.stderr)

        fit_input = fixture_input()
        no_references = {key: value for key, value in fit_input.items() if key != "references"}
        no_time = {**fit_input, "points": [{**fit_input["points"][0]}] + fit_input["points"][1:]}
        del no_time["points"][0]["time_seconds"]
        for name, data in [("no references", no_references), ("no time", no_time)]:
            with self.subTest(name):
                result = run_script("cold_resistance_fit.py", data)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("invalid input", result.stderr)


if __name__ == "__main__":
    unittest.main()
