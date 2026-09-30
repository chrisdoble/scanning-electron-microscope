"""Analyses one cold-resistance setpoint, measured in both polarities.

Reads a `ColdResistancePointInput` on stdin and prints a
`ColdResistancePointAnalysis` on stdout (see `schemas/`). See section 3.3 of
docs/COLD_RESISTANCE.md for the method.
"""

import math
import sys
from typing import Any

from uncertainties import UFloat, ufloat

from script_io import read_input, write_output


class AnalysisError(Exception):
    """The measurements can't be analysed. The message says why."""


def current_uncertainty(standard_error: float, resolution: float) -> float:
    """The standard uncertainty of a mean current, with a quantisation floor.

    The current is usually steadier than one readback step, so every sample
    reads the same and the standard error is 0, but the value is still only
    known to within one step: a rectangular distribution of full width
    `resolution`, whose standard deviation is `resolution / sqrt(12)`.
    """
    return math.sqrt(standard_error**2 + resolution**2 / 12)


def derived(quantity: UFloat) -> dict[str, float]:
    """`quantity` as a `Derived`."""
    return {"value": quantity.nominal_value, "uncertainty": quantity.std_dev}


def analyse(point: dict[str, Any]) -> dict[str, Any]:
    """Analyses a `ColdResistancePointInput`, returning a
    `ColdResistancePointAnalysis`.

    Raises `AnalysisError` if the measurements can't be analysed.
    """
    resolution = point["current_resolution_amps"]

    def current(measurement: dict[str, Any]) -> UFloat:
        return ufloat(
            measurement["value"],
            current_uncertainty(measurement["uncertainty"], resolution),
        )

    def voltage(measurement: dict[str, Any]) -> UFloat:
        return ufloat(measurement["value"], measurement["uncertainty"])

    v_p = voltage(point["positive_voltage_volts"])
    v_n = voltage(point["negative_voltage_volts"])
    i_p = current(point["positive_current_amps"])
    i_n = current(point["negative_current_amps"])

    # Otherwise the relay-to-sign mapping found while choosing the scale
    # didn't hold.
    if v_p.nominal_value <= 0:
        raise AnalysisError(
            f"the positive polarity's mean voltage isn't positive: {v_p.nominal_value} V"
        )
    if v_n.nominal_value >= 0:
        raise AnalysisError(
            f"the negative polarity's mean voltage isn't negative: {v_n.nominal_value} V"
        )

    # Voltages that don't reverse with the current (the scope's offset,
    # thermal EMFs, constant pickup) cancel in the difference. Both current
    # readbacks are positive, because the supply always sources positive
    # current and the relays do the reversal.
    v = (v_p - v_n) / 2
    i = (i_p + i_n) / 2
    r = v / i
    x = i**2

    warnings = []

    offset = (v_p + v_n) / 2
    if abs(offset.nominal_value) > point["offset_warning_volts"]:
        warnings.append(
            f"the voltage that doesn't reverse with the current, "
            f"{offset.nominal_value * 1000:.3f} mV, is more than "
            f"{point['offset_warning_volts'] * 1000:.3f} mV"
        )

    if v.nominal_value <= 0:
        raise AnalysisError(f"the voltage isn't positive: {v.nominal_value} V")

    # A zero uncertainty would give the point infinite weight in the fit.
    if r.std_dev == 0:
        raise AnalysisError("the resistance's uncertainty is 0")

    return {
        "current_amps": derived(i),
        "current_squared_amps_squared": derived(x),
        "offset_voltage_volts": derived(offset),
        "resistance_ohms": derived(r),
        "voltage_volts": derived(v),
        "warnings": warnings,
    }


def main() -> int:
    point = read_input("cold_resistance_point")

    try:
        analysis = analyse(point)
    except AnalysisError as e:
        print(e, file=sys.stderr)
        return 1

    write_output(analysis)
    return 0


if __name__ == "__main__":
    sys.exit(main())
