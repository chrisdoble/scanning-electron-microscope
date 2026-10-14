"""What the analysis scripts share: their error, the current's uncertainty, and
converting a result to a `Derived`.
"""

import math

from uncertainties import UFloat


class AnalysisError(Exception):
    """The input can't be analysed. The message says why."""


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
