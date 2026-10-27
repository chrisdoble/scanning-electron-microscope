"""Tests for the pulse binary's analysis script.

Run from this directory with `python -m unittest`.
"""

import json
import subprocess
import sys
import unittest
from pathlib import Path
from typing import Any

import jsonschema
import numpy as np

import pulse
from analysis import AnalysisError

PYTHON_DIR = Path(__file__).parent

PROBE_CURRENT = 0.1
BASELINE_RESISTANCE = 0.0977
EXTRA_AT_END = 1.5e-3
TIME_CONSTANT = 120.0
UP = 62.0
DOWN = 64.0


def probe_samples(
    start: float, end: float, forward_sign: float, noise: float, rng: np.random.Generator
) -> list[dict[str, Any]]:
    """Probe samples from `start` to `end`, in 10 s blocks alternating from
    forward, with the mount warmed by `EXTRA_AT_END` at `DOWN` and cooling with
    `TIME_CONSTANT`.

    Forward polarity gives a voltage of sign `forward_sign`. A drifting offset
    is added to every voltage, which reversal should cancel.
    """
    samples = []
    block_start = start
    polarity = "Forward"
    while block_start < end:
        sign = forward_sign if polarity == "Forward" else -forward_sign
        # Sampling starts a second into the block, as after a reversal.
        for t in np.arange(block_start + 1.0, min(block_start + 10.0, end), 0.4):
            resistance = BASELINE_RESISTANCE
            if t >= DOWN:
                resistance += EXTRA_AT_END * np.exp(-(t - DOWN) / TIME_CONSTANT)
            offset = -3e-4 + 2e-8 * t
            samples.append(
                {
                    "current_amps": PROBE_CURRENT,
                    "polarity": polarity,
                    "time_seconds": float(t),
                    "voltage_volts": float(
                        sign * resistance * PROBE_CURRENT + offset + rng.normal(0, noise)
                    ),
                }
            )
        block_start += 10.0
        polarity = "Reverse" if polarity == "Forward" else "Forward"
    return samples


def run_input(forward_sign: float = 1.0, noise: float = 5e-6, seed: int = 0) -> dict[str, Any]:
    """A `PulseAnalysisInput` for a 2 s pulse at a steady 2 W, with a 60 s
    baseline and a 600 s cool-down."""
    rng = np.random.default_rng(seed)
    return {
        "baseline": probe_samples(0.0, 60.0, forward_sign, noise, rng),
        "cold_resistance_ohms": 0.0977,
        "cool_down": probe_samples(DOWN, DOWN + 600.0, forward_sign, noise, rng),
        "fit_skip_seconds": 5.0,
        "pulse_down_seconds": DOWN,
        "pulse_samples": [
            {"current_amps": 2.0, "time_seconds": float(t), "voltage_volts": forward_sign * 1.0}
            for t in np.arange(UP + 0.4, DOWN, 0.4)
        ],
        "pulse_up_seconds": UP,
        "temperature_coefficients_per_kelvin": {"tungsten": 0.0045, "tungsten_rhenium": 0.0031},
    }


def within(test: unittest.TestCase, derived: dict[str, float], expected: float) -> None:
    """Checks `derived` is within four of its uncertainties of `expected`."""
    test.assertLess(abs(derived["value"] - expected), 4 * derived["uncertainty"], derived)


class TestPulse(unittest.TestCase):
    def test_recovers_the_warming_whichever_polarity_reads_positive(self) -> None:
        for forward_sign in [1.0, -1.0]:
            with self.subTest(forward_sign=forward_sign):
                analysis = pulse.analyse(run_input(forward_sign))
                fit = analysis["fit"]
                within(self, fit["extra_resistance_at_end_ohms"], EXTRA_AT_END)
                within(self, fit["time_constant_seconds"], TIME_CONSTANT)
                within(self, fit["baseline_resistance_ohms"], BASELINE_RESISTANCE)
                within(self, analysis["baseline"]["resistance_ohms"], BASELINE_RESISTANCE)
                within(self, analysis["baseline"]["slope_ohms_per_second"], 0.0)
                self.assertLess(fit["reduced_chi_squared"], 2.0)
                self.assertEqual(analysis["warnings"], [])

                # The uncertainty is small enough to plan with.
                self.assertLess(fit["extra_resistance_at_end_ohms"]["uncertainty"], 1e-4)

    def test_the_first_reading_is_a_lower_bound(self) -> None:
        analysis = pulse.analyse(run_input())
        first = analysis["first_reading"]
        self.assertGreaterEqual(first["seconds_after_pulse"], 5.0)
        self.assertLess(
            first["extra_resistance_ohms"]["value"],
            analysis["fit"]["extra_resistance_at_end_ohms"]["value"],
        )

    def test_the_pulse_energy_and_power(self) -> None:
        analysis = pulse.analyse(run_input(forward_sign=-1.0))
        summary = analysis["pulse"]
        self.assertAlmostEqual(summary["duration_seconds"], 2.0)
        self.assertAlmostEqual(summary["energy_joules"], 4.0)
        self.assertAlmostEqual(summary["mean_power_watts"], 2.0)
        self.assertAlmostEqual(summary["peak_power_watts"], 2.0)
        self.assertAlmostEqual(summary["final_resistance_ohms"], 0.5)

    def test_kelvin_follow_from_the_fitted_warming(self) -> None:
        analysis = pulse.analyse(run_input())
        extra = analysis["fit"]["extra_resistance_at_end_ohms"]["value"]
        rise = analysis["clamp_rise_kelvin"]
        self.assertAlmostEqual(rise["tungsten_rhenium"]["value"], extra / (0.0977 * 0.0031))
        self.assertAlmostEqual(rise["tungsten"]["value"], extra / (0.0977 * 0.0045))
        per_joule = analysis["kelvin_per_joule"]["tungsten_rhenium"]["value"]
        self.assertAlmostEqual(per_joule, rise["tungsten_rhenium"]["value"] / 4.0)
        self.assertAlmostEqual(
            analysis["budget_energy_joules"]["value"], pulse.CLAMP_RISE_BUDGET_KELVIN / per_joule
        )

    def test_neighbouring_pairs_are_correlated(self) -> None:
        resistances = pulse.probe_resistances(run_input()["baseline"])
        covariance = resistances.covariance
        correlation = covariance[0, 1] / np.sqrt(covariance[0, 0] * covariance[1, 1])
        self.assertGreater(correlation, 0.2)
        self.assertLess(correlation, 0.8)
        self.assertEqual(covariance[0, 2], 0.0)

    def test_too_short_a_cool_down_is_an_error(self) -> None:
        data = run_input()
        data["cool_down"] = [s for s in data["cool_down"] if s["time_seconds"] < DOWN + 30]
        with self.assertRaises(AnalysisError):
            pulse.analyse(data)

    def test_the_script_reads_and_writes_its_schemas(self) -> None:
        result = subprocess.run(
            [sys.executable, str(PYTHON_DIR / "pulse.py")],
            input=json.dumps(run_input()),
            capture_output=True,
            text=True,
            cwd=PYTHON_DIR,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        schema = json.loads((PYTHON_DIR / "schemas" / "pulse.output.json").read_text())
        jsonschema.validate(json.loads(result.stdout), schema)


if __name__ == "__main__":
    unittest.main()
