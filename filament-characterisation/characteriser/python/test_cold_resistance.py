"""Tests for the cold-resistance analysis scripts.

Run from this directory with `python -m unittest`. The cases are the ones in
the "Tests" section of docs/COLD_RESISTANCE.md.
"""

import json
import math
import subprocess
import sys
import unittest
from pathlib import Path
from typing import Any

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


def fixture_points(voltage_scale: float = 1.0, current_scale: float = 1.0) -> list[dict[str, Any]]:
    """The fixture's points as fit input, with every voltage and current
    optionally scaled."""
    return [
        {
            "voltage_volts": {
                "value": (R0 + B * i**2) * i * voltage_scale,
                "uncertainty": VOLTAGE_UNCERTAINTY,
            },
            "current_amps": {"value": i * current_scale, "uncertainty": CURRENT_UNCERTAINTY},
        }
        for i in SETPOINTS
    ]


def fit_fixture(points: list[dict[str, Any]]) -> cold_resistance_fit.Fit:
    data = cold_resistance_fit.to_points(points, 0.0)
    return cold_resistance_fit.fit_line(data.x, data.r, data.u_r)


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
        fit = fit_fixture(fixture_points())
        self.assertAlmostEqual(fit.r0, R0, delta=R0 * 1e-9)
        self.assertAlmostEqual(fit.chi_squared, 0.0, delta=1e-12)

    def test_gain_invariance(self) -> None:
        epsilon = 0.01
        fit = fit_fixture(fixture_points(voltage_scale=1 + epsilon))
        self.assertAlmostEqual(fit.r0, R0 * (1 + epsilon), delta=R0 * 1e-9)

        g = 0.0015
        fit = fit_fixture(fixture_points(current_scale=1 + g))
        self.assertAlmostEqual(fit.r0, R0 / (1 + g), delta=R0 * 1e-9)

    def test_offset_corners(self) -> None:
        lower, upper = cold_resistance_fit.offset_corners(fixture_points(), 0.005)
        self.assertAlmostEqual(upper, 0.099849, delta=0.099849 * 1e-5)
        self.assertAlmostEqual(lower, 0.092317, delta=0.092317 * 1e-5)

        analysis = cold_resistance_fit.analyse({"parameters": PARAMETERS, "points": fixture_points()})
        self.assertAlmostEqual(
            analysis["current_offset_uncertainty_ohms"], 0.002174, delta=0.002174 * 1e-4
        )

    def test_temperature_correction(self) -> None:
        analysis = cold_resistance_fit.analyse({"parameters": PARAMETERS, "points": fixture_points()})
        self.assertAlmostEqual(
            analysis["reference_resistance_ohms"]["value"], 0.093888, delta=0.093888 * 1e-5
        )
        self.assertAlmostEqual(analysis["temperature_uncertainty_ohms"], 2.39e-4, delta=2.39e-4 * 5e-3)
        self.assertAlmostEqual(
            analysis["temperature_coefficient_uncertainty_ohms"], 7.95e-5, delta=7.95e-5 * 5e-3
        )

        at_reference = cold_resistance_fit.analyse(
            {
                "parameters": {**PARAMETERS, "filament_temperature_celsius": 20.0},
                "points": fixture_points(),
            }
        )
        self.assertEqual(
            at_reference["reference_resistance_ohms"]["value"],
            at_reference["resistance_ohms"]["value"],
        )
        self.assertEqual(at_reference["temperature_coefficient_uncertainty_ohms"], 0.0)

    def test_matches_closed_form(self) -> None:
        # Noisy data with unequal uncertainties, fitted by curve_fit and by the
        # closed-form weighted least squares. A missing `absolute_sigma=True`
        # would scale the fit's uncertainties by the square root of the
        # reduced chi-squared.
        rng = np.random.default_rng(1)
        x = np.array(SETPOINTS) ** 2
        u_r = rng.uniform(5e-6, 30e-6, len(x))
        r = R0 + B * x + rng.normal(0, u_r)

        fit = cold_resistance_fit.fit_line(x, r, u_r)

        w = 1 / u_r**2
        s, s_x, s_y = w.sum(), (w * x).sum(), (w * r).sum()
        s_xx, s_xy = (w * x * x).sum(), (w * x * r).sum()
        delta = s * s_xx - s_x**2
        expected = {
            "r0": (s_xx * s_y - s_x * s_xy) / delta,
            "b": (s * s_xy - s_x * s_y) / delta,
            "u_r0": math.sqrt(s_xx / delta),
            "u_b": math.sqrt(s / delta),
        }
        for name, value in expected.items():
            with self.subTest(name):
                self.assertAlmostEqual(getattr(fit, name), value, delta=abs(value) * 1e-5)


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
        self.assert_output_valid(
            "cold_resistance_fit", {"parameters": PARAMETERS, "points": fixture_points()}
        )

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


if __name__ == "__main__":
    unittest.main()
