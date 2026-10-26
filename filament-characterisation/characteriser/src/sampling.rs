//! What every binary that takes samples of the filament uses, whether or not it
//! has the characteriser's terminal UI: choosing a vertical scale, judging
//! clipping and summarising readings, with the constants they use.

use host::oscilloscope::Voltages;
use std::time::Duration;

/// A reading counts as clipped if the voltage's peak is more than this
/// fraction of the way to the edge of the screen (4 divisions from the
/// centre).
///
/// The scale is chosen once, then fixed, and later readings are bigger: the
/// filament's resistance keeps rising as it settles, a later window can catch a
/// larger noise peak, and offsets drift. Without margin, a scale that only just
/// fits would clip mid-run and abort it. The margin costs almost nothing,
/// because the gain uncertainty doesn't depend on the scale.
pub const CLIPPING_HEADROOM: f64 = 0.8;

/// How far above the largest setpoint the supply's overcurrent protection is
/// set, in amperes, as a hardware backstop.
///
/// Its 10 ms delay should ride out any overshoot when the output is re-enabled
/// at a setpoint. If it trips anyway, widen this or lengthen the delay.
pub const OVERCURRENT_PROTECTION_MARGIN_AMPS: f64 = 0.05;

/// How long the output stays off while the polarity relays move.
pub const RELAY_SETTLE_TIME: Duration = Duration::from_millis(50);

/// The oscilloscope's vertical scales in volts per division, smallest first.
///
/// Starts at 10 mV/div, because the ±1% gain spec is only "typical" at 5 mV/div
/// and below.
pub const VERTICAL_SCALES_VOLTS_PER_DIVISION: [f64; 10] =
    [0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0];

/// The mean and standard error of `values`, or `None` if there are fewer than
/// two.
pub fn mean_and_standard_error(values: &[f64]) -> Option<(f64, f64)> {
    if values.len() < 2 {
        return None;
    }

    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0);
    Some((mean, (variance / n).sqrt()))
}

/// The average of `voltages`, or `None` if it's clipped on `scale`: if any
/// value is invalid, or the peak is beyond `CLIPPING_HEADROOM` of the way to
/// the edge of the screen.
pub fn unclipped_average(voltages: Voltages, scale: f64) -> Option<f64> {
    let limit = CLIPPING_HEADROOM * 4.0 * scale;
    match voltages {
        Voltages {
            average: Some(average),
            maximum: Some(maximum),
            minimum: Some(minimum),
        } if maximum.abs().max(minimum.abs()) <= limit => Some(average),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A reading is clipped if any value is invalid or its peak is past the
    // headroom, even when the average is fine.
    #[test]
    fn unclipped_average_rejects_any_clipped_value() {
        let scale = 0.01;
        let limit = CLIPPING_HEADROOM * 4.0 * scale;
        let voltages = |maximum, minimum| Voltages {
            average: Some(0.001),
            maximum,
            minimum,
        };

        assert_eq!(
            unclipped_average(voltages(Some(limit), Some(-limit)), scale),
            Some(0.001)
        );
        assert_eq!(
            unclipped_average(voltages(Some(limit * 1.01), Some(0.0)), scale),
            None
        );
        assert_eq!(
            unclipped_average(voltages(Some(0.0), Some(-limit * 1.01)), scale),
            None
        );
        assert_eq!(unclipped_average(voltages(None, Some(0.0)), scale), None);
        assert_eq!(unclipped_average(voltages(Some(0.0), None), scale), None);
        assert_eq!(
            unclipped_average(
                Voltages {
                    average: None,
                    maximum: Some(0.0),
                    minimum: Some(0.0),
                },
                scale
            ),
            None
        );
    }
}
