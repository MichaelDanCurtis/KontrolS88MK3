//! The authoring model: what a user or the UI describes.

use crate::{DeviceId, NodeId, ParamName};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LfoShape {
    Triangle,
    Sine,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NodeSpec {
    Lfo { shape: LfoShape, freq_hz: f64 },
    Scale { min: f32, max: f32 },
    Sink { device: DeviceId, param: ParamName },
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
