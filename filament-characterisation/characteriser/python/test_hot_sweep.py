"""Tests for the hot sweep's analysis script.

Run from this directory with `python -m unittest`. The cases are the ones in the
"Tests" section of `filament-characterisation/docs/5_HOT_SWEEP.md`.
"""

import json
import math
import subprocess
import sys
import unittest
from pathlib import Path
from typing import Any

import jsonschema

import hot_sweep_point

PYTHON_DIR = Path(__file__).parent
RESOLUTION = 1e-4


def measurement(value: float, uncertainty: float) -> dict[str, Any]:
    return {"samples": [value, value], "value": value, "uncertainty": uncertainty}


def point_input(
    voltage: float = 0.1,
    offset: float = 0.002,
    voltage_uncertainty: float = 1e-5,
    offset_uncertainty: float = 2e-5,
    current_uncertainty: float = 3e-5,
) -> dict[str, Any]:
    """A `HotSweepPointInput` at 0.5 A."""
    return {
        "current_amps": measurement(0.5, current_uncertainty),
        "current_resolution_amps": RESOLUTION,
        "offset_voltage_volts": measurement(offset, offset_uncertainty),
        "voltage_volts": measurement(voltage, voltage_uncertainty),
    }


def run_script(data: Any) -> subprocess.CompletedProcess:
    """Runs `hot_sweep_point.py` with `data` as JSON on stdin."""
    return subprocess.run(
        [sys.executable, str(PYTHON_DIR / "hot_sweep_point.py")],
        input=json.dumps(data),
        capture_output=True,
        text=True,
        cwd=PYTHON_DIR,
    )


class TestHotSweepPoint(unittest.TestCase):
    def test_offset_is_subtracted(self) -> None:
        analysis = hot_sweep_point.analyse(point_input())
        voltage = analysis["voltage_volts"]
        self.assertAlmostEqual(voltage["value"], 0.098, delta=1e-12)
        self.assertAlmostEqual(voltage["uncertainty"], math.sqrt(1e-5**2 + 2e-5**2), delta=1e-15)
        self.assertAlmostEqual(analysis["resistance_ohms"]["value"], 0.098 / 0.5, delta=1e-12)

    def test_sign_does_not_matter(self) -> None:
        # +0.1 - 0.002 and -0.096 - 0.002 are both 0.098 V in size.
        positive = hot_sweep_point.analyse(point_input(voltage=0.1))
        negative = hot_sweep_point.analyse(point_input(voltage=-0.096))
        for name in ("voltage_volts", "resistance_ohms"):
            with self.subTest(name):
                self.assertAlmostEqual(negative[name]["value"], positive[name]["value"], delta=1e-12)
                self.assertAlmostEqual(
                    negative[name]["uncertainty"], positive[name]["uncertainty"], delta=1e-15
                )

    def test_quantisation_floor(self) -> None:
        analysis = hot_sweep_point.analyse(point_input(current_uncertainty=0.0))
        expected = RESOLUTION / math.sqrt(12)
        self.assertAlmostEqual(analysis["current_amps"]["uncertainty"], expected, delta=expected * 1e-12)

    def test_errors(self) -> None:
        cases = [
            ("voltage equal to its offset", point_input(voltage=0.002), "is 0"),
            (
                "zero uncertainty",
                {
                    **point_input(voltage_uncertainty=0.0, offset_uncertainty=0.0, current_uncertainty=0.0),
                    "current_resolution_amps": 0.0,
                },
                "uncertainty is 0",
            ),
        ]
        for name, data, message in cases:
            with self.subTest(name):
                result = run_script(data)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(message, result.stderr)


class TestHotSweepSchemas(unittest.TestCase):
    def test_output_matches_its_schema(self) -> None:
        result = run_script(point_input())
        self.assertEqual(result.returncode, 0, result.stderr)
        schema = json.loads((PYTHON_DIR / "schemas" / "hot_sweep_point.output.json").read_text())
        jsonschema.validate(json.loads(result.stdout), schema)

    def test_invalid_input_is_rejected(self) -> None:
        missing = point_input()
        del missing["offset_voltage_volts"]
        unknown = {**point_input(), "unexpected": 1}
        wrong_type = {**point_input(), "current_resolution_amps": "0.0001"}

        for name, data in [("missing", missing), ("unknown", unknown), ("wrong type", wrong_type)]:
            with self.subTest(name):
                result = run_script(data)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("invalid input", result.stderr)


if __name__ == "__main__":
    unittest.main()
