"""Summarises a set of samples as a value and an uncertainty.

Reads one JSON object on stdin, with at least two samples:

    {"samples": [<sample>, ...]}

and prints one JSON object on stdout:

    {"value": <mean>, "uncertainty": <standard error of the mean>}

TODO: resistance and its propagated uncertainty are calculated by scripts added
beside this one.
"""

import math
import statistics
import sys

from uncertainties import ufloat

from script_io import read_input, write_output


def main() -> int:
    samples = read_input("mean_and_standard_error")["samples"]

    # The standard error of the mean needs the sample standard deviation, which
    # isn't defined for fewer than two samples.
    if len(samples) < 2:
        print(
            f"expected at least 2 samples, got {len(samples)}",
            file=sys.stderr,
        )
        return 1

    # Carrying the pair as a `ufloat` isn't needed for a mean, but it's how the
    # scripts that propagate uncertainties will work, so exercise it here too.
    quantity = ufloat(
        statistics.fmean(samples),
        statistics.stdev(samples) / math.sqrt(len(samples)),
    )

    write_output({"value": quantity.nominal_value, "uncertainty": quantity.std_dev})
    return 0


if __name__ == "__main__":
    sys.exit(main())
