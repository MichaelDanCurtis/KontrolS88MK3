//! Node evaluation. Every function here is total: no panics, no unwraps,
//! no unguarded division, and no clock access.

use cr_core::LfoShape;

/// The engine's fixed control rate.
pub const TICK_HZ: f64 = 1000.0;

/// Compute an LFO output in `0.0..=1.0` from the absolute tick index.
///
/// Phase derives from the tick number rather than accumulating, so there is
/// no drift and the result is reproducible for any tick.
pub fn lfo_value(shape: LfoShape, freq_hz: f64, tick: u64) -> f32 {
    // `compile()` rejects non-positive and non-finite frequencies, but this
    // function stays total regardless of how it is called.
    if !freq_hz.is_finite() || freq_hz <= 0.0 {
        return 0.0;
    }
    let phase = ((tick as f64) * freq_hz / TICK_HZ).rem_euclid(1.0);
    let v = match shape {
        LfoShape::Triangle => {
            if phase < 0.5 {
                2.0 * phase
            } else {
                2.0 - 2.0 * phase
            }
        }
        LfoShape::Sine => 0.5 - 0.5 * (std::f64::consts::TAU * phase).cos(),
    };
    v.clamp(0.0, 1.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-6;

    #[test]
    fn triangle_hits_known_points_of_one_cycle() {
        // 1 Hz at 1 kHz => one full cycle per 1000 ticks.
        assert!((lfo_value(LfoShape::Triangle, 1.0, 0) - 0.0).abs() < EPS);
        assert!((lfo_value(LfoShape::Triangle, 1.0, 250) - 0.5).abs() < EPS);
        assert!((lfo_value(LfoShape::Triangle, 1.0, 500) - 1.0).abs() < EPS);
        assert!((lfo_value(LfoShape::Triangle, 1.0, 750) - 0.5).abs() < EPS);
    }

    #[test]
    fn triangle_is_periodic_without_drift() {
        // The same phase 1000 cycles later must be identical, proving the
        // absolute-tick formulation does not accumulate error.
        let a = lfo_value(LfoShape::Triangle, 1.0, 250);
        let b = lfo_value(LfoShape::Triangle, 1.0, 250 + 1_000_000);
        assert!((a - b).abs() < EPS);
    }

    #[test]
    fn sine_hits_known_points_of_one_cycle() {
        assert!((lfo_value(LfoShape::Sine, 1.0, 0) - 0.0).abs() < EPS);
        assert!((lfo_value(LfoShape::Sine, 1.0, 500) - 1.0).abs() < EPS);
    }

    #[test]
    fn output_always_within_unit_range() {
        for tick in 0..5000u64 {
            let v = lfo_value(LfoShape::Sine, 0.3, tick);
            assert!((0.0..=1.0).contains(&v), "out of range at {tick}: {v}");
        }
    }

    #[test]
    fn invalid_frequency_returns_zero_without_panicking() {
        assert_eq!(lfo_value(LfoShape::Triangle, 0.0, 10), 0.0);
        assert_eq!(lfo_value(LfoShape::Triangle, -1.0, 10), 0.0);
        assert_eq!(lfo_value(LfoShape::Triangle, f64::NAN, 10), 0.0);
        assert_eq!(lfo_value(LfoShape::Triangle, f64::INFINITY, 10), 0.0);
    }
}
