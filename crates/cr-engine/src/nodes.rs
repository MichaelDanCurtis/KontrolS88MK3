//! Node evaluation. Every function here is total: no panics, no unwraps,
//! no unguarded division, and no clock access.

use cr_core::{LfoShape, MathOperation};

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
    // For a finite, positive but astronomically large `freq_hz`, the product
    // above can overflow to infinity, and `rem_euclid` on infinity yields
    // NaN. `f64::clamp` does not rescue NaN (both its comparisons are
    // false), so guard explicitly to keep this function total.
    if !phase.is_finite() {
        return 0.0;
    }
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

fn period_index(rate_hz: f64, tick: u64) -> Option<u64> {
    if !rate_hz.is_finite() || rate_hz <= 0.0 {
        return None;
    }
    let index = ((tick as f64) * rate_hz / TICK_HZ).floor();
    index.is_finite().then_some(index as u64)
}

/// Return a deterministic pseudo-random value in `0.0..=1.0`, held for one
/// period. The value depends only on the absolute tick, rate, and seed.
pub fn sample_hold_value(rate_hz: f64, seed: u64, tick: u64) -> f32 {
    let Some(period) = period_index(rate_hz, tick) else {
        return 0.0;
    };
    // SplitMix64 is compact, deterministic across platforms, and needs no
    // mutable random-number-generator state in the real-time engine.
    let mut value = period
        .wrapping_add(seed)
        .wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    ((value >> 40) as f32) / ((1_u32 << 24) - 1) as f32
}

/// Return the value for an absolute tick from a repeating step sequence.
pub fn step_sequence_value(rate_hz: f64, values: &[f32], tick: u64) -> f32 {
    let Some(period) = period_index(rate_hz, tick) else {
        return 0.0;
    };
    let Some(index) = usize::try_from(period % values.len().max(1) as u64).ok() else {
        return 0.0;
    };
    values
        .get(index)
        .copied()
        .filter(|v| v.is_finite())
        .unwrap_or(0.0)
}

pub fn clamp_value(input: f32, min: f32, max: f32) -> f32 {
    if !input.is_finite() || !min.is_finite() || !max.is_finite() || min >= max {
        return 0.0;
    }
    input.clamp(min, max)
}

pub fn curve_value(input: f32, exponent: f32) -> f32 {
    if !input.is_finite() || !exponent.is_finite() || exponent <= 0.0 {
        return 0.0;
    }
    input.clamp(0.0, 1.0).powf(exponent)
}

pub fn quantize_value(input: f32, steps: u16) -> f32 {
    if !input.is_finite() || steps < 2 {
        return 0.0;
    }
    let intervals = f32::from(steps - 1);
    (input.clamp(0.0, 1.0) * intervals).round() / intervals
}

pub fn math_value(input: f32, operation: MathOperation, operand: f32) -> f32 {
    if !input.is_finite() || !operand.is_finite() {
        return 0.0;
    }
    let output = match operation {
        MathOperation::Add => input + operand,
        MathOperation::Subtract => input - operand,
        MathOperation::Multiply => input * operand,
        MathOperation::Divide if operand != 0.0 => input / operand,
        MathOperation::Divide => return 0.0,
    };
    if output.is_finite() { output } else { 0.0 }
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
    fn triangle_output_always_within_unit_range() {
        for tick in 0..5000u64 {
            let v = lfo_value(LfoShape::Triangle, 0.3, tick);
            assert!((0.0..=1.0).contains(&v), "out of range at {tick}: {v}");
        }
    }

    #[test]
    fn astronomically_large_freq_does_not_produce_nan() {
        // A finite, positive but huge `freq_hz` overflows `tick * freq_hz` to
        // +inf; `rem_euclid(1.0)` on infinity yields NaN, which `f64::clamp`
        // does not rescue (both its comparisons are false against NaN).
        //
        // `tick` must be >= 2 here: `1.0 * f64::MAX` does not itself overflow
        // (multiplying by 1 leaves the magnitude unchanged), so a `tick` of 1
        // would compute a finite phase and never exercise the guard.
        // `2.0 * f64::MAX` does overflow to infinity, which is what drives
        // `rem_euclid` to NaN and requires the phase guard to catch it.
        for shape in [LfoShape::Triangle, LfoShape::Sine] {
            let v = lfo_value(shape, f64::MAX, 2);
            assert!(v.is_finite(), "expected finite output, got {v}");
            assert!((0.0..=1.0).contains(&v), "out of range: {v}");
        }
    }

    #[test]
    fn invalid_frequency_returns_zero_without_panicking() {
        assert_eq!(lfo_value(LfoShape::Triangle, 0.0, 10), 0.0);
        assert_eq!(lfo_value(LfoShape::Triangle, -1.0, 10), 0.0);
        assert_eq!(lfo_value(LfoShape::Triangle, f64::NAN, 10), 0.0);
        assert_eq!(lfo_value(LfoShape::Triangle, f64::INFINITY, 10), 0.0);
    }

    #[test]
    fn sample_hold_is_stable_within_a_period_and_seeded() {
        let first = sample_hold_value(4.0, 42, 0);
        assert_eq!(first, sample_hold_value(4.0, 42, 249));
        assert_ne!(first, sample_hold_value(4.0, 42, 250));
        assert_ne!(first, sample_hold_value(4.0, 43, 0));
        assert!((0.0..=1.0).contains(&first));
    }

    #[test]
    fn step_sequence_advances_and_wraps_from_absolute_ticks() {
        let values = [0.1, 0.5, 0.9];
        assert_eq!(step_sequence_value(2.0, &values, 0), 0.1);
        assert_eq!(step_sequence_value(2.0, &values, 499), 0.1);
        assert_eq!(step_sequence_value(2.0, &values, 500), 0.5);
        assert_eq!(step_sequence_value(2.0, &values, 1_000), 0.9);
        assert_eq!(step_sequence_value(2.0, &values, 1_500), 0.1);
    }

    #[test]
    fn new_sources_are_total_for_invalid_inputs() {
        assert_eq!(sample_hold_value(f64::NAN, 0, 0), 0.0);
        assert_eq!(step_sequence_value(1.0, &[], 0), 0.0);
        assert_eq!(step_sequence_value(1.0, &[f32::NAN], 0), 0.0);
    }

    #[test]
    fn transforms_have_known_values_and_total_failure_behavior() {
        assert_eq!(clamp_value(1.5, 0.0, 1.0), 1.0);
        assert_eq!(curve_value(0.5, 2.0), 0.25);
        assert_eq!(quantize_value(0.6, 3), 0.5);
        assert_eq!(math_value(0.5, MathOperation::Add, 0.25), 0.75);
        assert_eq!(math_value(0.5, MathOperation::Divide, 0.0), 0.0);
        assert_eq!(math_value(f32::MAX, MathOperation::Multiply, 2.0), 0.0);
        assert_eq!(curve_value(f32::NAN, 2.0), 0.0);
    }
}
