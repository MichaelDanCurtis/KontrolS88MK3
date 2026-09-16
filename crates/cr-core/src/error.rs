use crate::NodeId;
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum CompileError {
    #[error("graph is empty")]
    EmptyGraph,
    #[error("duplicate node id: {0}")]
    DuplicateNode(NodeId),
    #[error("node {node} references unknown input {input}")]
    UnknownInput { node: NodeId, input: NodeId },
    #[error("graph contains a cycle involving {0}")]
    Cycle(NodeId),
    #[error("node {node} has invalid lfo frequency {freq} (must be > 0 and finite)")]
    InvalidLfoFreq { node: NodeId, freq: f64 },
    #[error("node {node} has invalid sample-and-hold rate {rate} (must be > 0 and finite)")]
    InvalidSampleHoldRate { node: NodeId, rate: f64 },
    #[error("node {node} has invalid step-sequence rate {rate} (must be > 0 and finite)")]
    InvalidStepSequenceRate { node: NodeId, rate: f64 },
    #[error("node {node} has no step-sequence values")]
    EmptyStepSequence { node: NodeId },
    #[error("node {node} has too many step-sequence values ({count}; maximum is {max})")]
    StepSequenceTooLong {
        node: NodeId,
        count: usize,
        max: usize,
    },
    #[error("node {node} has a non-finite step-sequence value at index {index}")]
    InvalidStepSequenceValue { node: NodeId, index: usize },
    #[error("node {node} has invalid slew rates: rise {rise}, fall {fall}")]
    InvalidSlewRate { node: NodeId, rise: f32, fall: f32 },
    #[error("node {node} has invalid clamp range: min {min} max {max}")]
    InvalidClampRange { node: NodeId, min: f32, max: f32 },
    #[error("node {node} has invalid curve exponent {exponent} (must be > 0 and finite)")]
    InvalidCurveExponent { node: NodeId, exponent: f32 },
    #[error("node {node} has invalid quantize step count {steps} (must be at least 2)")]
    InvalidQuantizeSteps { node: NodeId, steps: u16 },
    #[error("node {node} has invalid math operand {operand}")]
    InvalidMathOperand { node: NodeId, operand: f32 },
    #[error("node {node} divides by zero")]
    DivisionByZero { node: NodeId },
    #[error("envelope node {node} references unknown gate {gate}")]
    UnknownGate { node: NodeId, gate: NodeId },
    #[error(
        "envelope node {node} has invalid ADSR values: attack {attack}, decay {decay}, sustain {sustain}, release {release}"
    )]
    InvalidEnvelope {
        node: NodeId,
        attack: f32,
        decay: f32,
        sustain: f32,
        release: f32,
    },
    #[error("node {node} has invalid scale range: min {min} max {max}")]
    InvalidScaleRange { node: NodeId, min: f32, max: f32 },
    #[error("node {node} has an empty external binding")]
    EmptyBinding { node: NodeId },
    #[error("node {node} has invalid external default {value} (must be finite)")]
    InvalidExternalDefault { node: NodeId, value: f32 },
    #[error("takeover node {node} references unknown control {control}")]
    UnknownControl { node: NodeId, control: NodeId },
    #[error("takeover node {node} has invalid pickup epsilon {value}")]
    InvalidPickupEpsilon { node: NodeId, value: f32 },
}
