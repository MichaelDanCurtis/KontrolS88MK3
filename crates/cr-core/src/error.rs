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
    #[error("node {node} has invalid scale range: min {min} max {max}")]
    InvalidScaleRange { node: NodeId, min: f32, max: f32 },
}
