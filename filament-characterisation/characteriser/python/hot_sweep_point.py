"""Analyses one hot sweep point, measured in one polarity.

Reads a `HotSweepPointInput` on stdin and prints a `HotSweepPointAnalysis` on
stdout (see `schemas/`). See the "Analysis" section of
`filament-characterisation/docs/5_HOT_SWEEP.md` for the method.
"""

import sys
from typing import Any

from uncertainties import ufloat

from cold_resistance_point import AnalysisError, current_uncertainty, derived
from script_io import read_input, write_output


def analyse(point: dict[str, Any]) -> dict[str, Any]:
    """Analyses a `HotSweepPointInput`, returning a `HotSweepPointAnalysis`.

    Raises `AnalysisError` if the measurements can't be analysed.
    """

    def measured(measurement: dict[str, Any]) -> Any:
        return ufloat(measurement["value"], measurement["uncertainty"])

    # Without reversal, the scope's offset is subtracted instead: its reading
    # with no current on the same scale.
    v = measured(point["voltage_volts"]) - measured(point["offset_voltage_volts"])
    if v.nominal_value == 0:
        raise AnalysisError("the voltage less the scope's offset is 0")

    # A magnitude, because the sign depends on how the relays and sense leads
    # are wired, which the sweep doesn't need to know.
    v = abs(v)

    i = ufloat(
        point["current_amps"]["value"],
        current_uncertainty(point["current_amps"]["uncertainty"], point["current_resolution_amps"]),
    )
    r = v / i

    if r.std_dev == 0:
        raise AnalysisError("the resistance's uncertainty is 0")

    return {
        "current_amps": derived(i),
        "resistance_ohms": derived(r),
        "voltage_volts": derived(v),
    }


def main() -> int:
    point = read_input("hot_sweep_point")

    try:
        analysis = analyse(point)
    except AnalysisError as e:
        print(e, file=sys.stderr)
        return 1

    write_output(analysis)
    return 0


if __name__ == "__main__":
    sys.exit(main())
