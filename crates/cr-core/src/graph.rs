//! The authoring model: what a user or the UI describes.

use crate::{BindingId, DeviceId, NodeId, ParamName};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LfoShape {
    Triangle,
    Sine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MathOperation {
    Add,
    Subtract,
    Multiply,
    Divide,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NodeSpec {
    Lfo {
        shape: LfoShape,
        freq_hz: f64,
    },
    /// Deterministic pseudo-random value held for each period.
    SampleHold {
        rate_hz: f64,
        seed: u64,
    },
    /// Repeating values advanced at `rate_hz` steps per second.
    StepSequence {
        rate_hz: f64,
        values: Vec<f32>,
    },
    /// Limits how quickly a signal can rise and fall.
    Slew {
        rise_per_second: f32,
        fall_per_second: f32,
    },
    Clamp {
        min: f32,
        max: f32,
    },
    /// Applies a unipolar power curve to an input in `0.0..=1.0`.
    Curve {
        exponent: f32,
    },
    /// Snaps a unipolar signal to an evenly-spaced set of values.
    Quantize {
        steps: u16,
    },
    Math {
        operation: MathOperation,
        operand: f32,
    },
    /// ADSR envelope driven by a graph node interpreted as a gate.
    Envelope {
        gate: NodeId,
        attack_seconds: f32,
        decay_seconds: f32,
        sustain: f32,
        release_seconds: f32,
    },
    /// Named human-facing control supplied through an external adapter.
    Macro {
        binding: BindingId,
        default: f32,
    },
    Scale {
        min: f32,
        max: f32,
    },
    Sink {
        device: DeviceId,
        param: ParamName,
    },
    /// Value supplied by an adapter outside the deterministic engine.
    ExternalSource {
        binding: BindingId,
        default: f32,
    },
    /// Value emitted to an adapter outside the deterministic engine.
    ExternalSink {
        binding: BindingId,
    },
    /// Follows `input` until the physical control crosses the current value,
    /// then follows that control without a jump.
    Takeover {
        control: NodeId,
        pickup_epsilon: f32,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: NodeId,
    pub spec: NodeSpec,
    pub input: Option<NodeId>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
}
