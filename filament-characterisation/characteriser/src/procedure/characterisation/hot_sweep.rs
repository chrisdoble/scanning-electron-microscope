//! Measuring the hot sweep: the filament's resistance at setpoints up to a
//! maximum current and back down again. See
//! `filament-characterisation/docs/5_HOT_SWEEP.md`.

/// The step in I², in A², and the currents to measure going up, in amperes:
/// 100 mA, then a grid evenly spaced in I² up to `maximum`, which must be above
/// 100 mA.
///
/// The grid is the multiples of the step, so runs to different maxima share
/// setpoints. The step is the smallest of `STEPS_AMPS_SQUARED` that keeps the
/// grid to at most `MAXIMUM_POINTS` points, so the sweep stays short however
/// high the maximum: a longer sweep would also warm the mount more. Above about
/// 2 A even the largest step gives more points than that, and it's used anyway.
/// Every step divides 0.2 A², so multiples of it are setpoints at any maximum.
///
/// 100 mA comes first whatever the step, because it's where the filament's own
/// heating is smallest, so its up/down difference best shows how much the mount
/// warmed. The maximum is always the last point, added if it isn't on the grid.
// Only the tests use it until the procedure does, so only expect it unused
// outside them.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "used by the hot sweep section, added in step 4 of 5_HOT_SWEEP.md"
    )
)]
pub(super) fn hot_sweep_setpoints(maximum: f64) -> (f64, Vec<f64>) {
    const STEPS_AMPS_SQUARED: [f64; 5] = [0.01, 0.02, 0.05, 0.1, 0.2];
    const MAXIMUM_POINTS: usize = 20;
    const LOWEST_AMPS: f64 = 0.1;

    // In A². Floating point puts some multiples a hair off: 0.6² / 0.02 is
    // 17.999… in f64, but 0.36 A² is on the grid.
    const TOLERANCE: f64 = 1e-9;

    let maximum_squared = maximum * maximum;

    // The grid for `step`: its multiples above 100 mA, up to the maximum.
    let grid = |step: f64| -> Vec<f64> {
        let last = (maximum_squared / step + TOLERANCE).floor() as usize;
        let mut currents: Vec<f64> = (1..=last)
            .map(|k| k as f64 * step)
            .filter(|&squared| squared > LOWEST_AMPS * LOWEST_AMPS + TOLERANCE)
            .map(f64::sqrt)
            .collect();

        // Exactly the maximum, rather than a square root that rounds near it.
        if (last as f64 * step - maximum_squared).abs() < TOLERANCE {
            if let Some(top) = currents.last_mut() {
                *top = maximum;
            }
        } else {
            currents.push(maximum);
        }
        currents
    };

    let step = STEPS_AMPS_SQUARED
        .into_iter()
        .find(|&step| grid(step).len() <= MAXIMUM_POINTS)
        .unwrap_or(STEPS_AMPS_SQUARED[STEPS_AMPS_SQUARED.len() - 1]);

    let mut setpoints = vec![LOWEST_AMPS];
    setpoints.extend(grid(step));
    (step, setpoints)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether `a` and `b` are equal to within floating-point rounding.
    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn hot_sweep_setpoints_to_0_6_amps() {
        let (step, setpoints) = hot_sweep_setpoints(0.6);
        assert_eq!(step, 0.02);
        assert_eq!(setpoints.len(), 19);
        assert_eq!(setpoints[0], 0.1);
        assert!(close(setpoints[1], 0.02_f64.sqrt()));
        assert_eq!(setpoints[18], 0.6);
    }

    #[test]
    fn hot_sweep_setpoints_to_1_amp() {
        let (step, setpoints) = hot_sweep_setpoints(1.0);
        assert_eq!(step, 0.05);
        assert_eq!(setpoints.len(), 21);
        assert_eq!(setpoints.last(), Some(&1.0));
    }

    #[test]
    fn hot_sweep_setpoints_to_1_6_amps() {
        let (step, setpoints) = hot_sweep_setpoints(1.6);
        assert_eq!(step, 0.2);
        assert_eq!(setpoints.len(), 14);
        assert!(close(setpoints[1], 0.2_f64.sqrt()));
        assert_eq!(setpoints.last(), Some(&1.6));
    }

    #[test]
    fn hot_sweep_setpoints_start_at_100_milliamps_once() {
        // With a 0.01 A² step, the grid's first multiple is 100 mA itself.
        let (step, setpoints) = hot_sweep_setpoints(0.4);
        assert_eq!(step, 0.01);
        assert_eq!(setpoints[0], 0.1);
        assert!(setpoints[1] > 0.1);
        assert_eq!(setpoints.len(), 16);
    }

    #[test]
    fn hot_sweep_setpoints_end_on_a_maximum_off_the_grid() {
        // 0.55² is 0.3025 A², between multiples of 0.02 A².
        let (step, setpoints) = hot_sweep_setpoints(0.55);
        assert_eq!(step, 0.02);
        assert_eq!(setpoints.last(), Some(&0.55));
        assert!(close(setpoints[setpoints.len() - 2], 0.30_f64.sqrt()));
    }

    #[test]
    fn hot_sweep_setpoints_increase_within_the_point_limit_and_nest() {
        // Every maximum in 10 mA steps from 110 mA to 2 A.
        for centiamps in 11..=200 {
            let maximum = f64::from(centiamps) / 100.0;
            let (step, setpoints) = hot_sweep_setpoints(maximum);

            assert!(
                setpoints.windows(2).all(|pair| pair[0] < pair[1]),
                "{} A: {:?}",
                maximum,
                setpoints
            );
            assert!(setpoints.len() - 1 <= 20, "{} A: {:?}", maximum, setpoints);
            assert_eq!(setpoints.last(), Some(&maximum));

            // Every multiple of 0.2 A² up to the maximum is a setpoint.
            let mut multiple = 0.2;
            while multiple <= maximum * maximum + 1e-9 {
                assert!(
                    setpoints
                        .iter()
                        .any(|&current| close(current * current, multiple)),
                    "{} A with a {} A² step is missing {} A²",
                    maximum,
                    step,
                    multiple
                );
                multiple += 0.2;
            }
        }
    }
}
