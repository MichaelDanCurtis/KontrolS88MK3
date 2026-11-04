//! Validation and topological ordering.
//!
//! `CompiledGraph` has private fields and no public constructor, so the only
//! way to obtain one is through `compile()`. The engine therefore cannot be
//! handed an invalid graph.

use crate::NodeId;
use crate::error::CompileError;
use crate::graph::{Graph, MathOperation, NodeSpec};
use std::collections::{HashMap, VecDeque};

pub const MAX_STEP_SEQUENCE_VALUES: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub struct CompiledNode {
    pub spec: NodeSpec,
    pub input: Option<usize>,
    pub control: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompiledGraph {
    nodes: Vec<CompiledNode>,
    order: Vec<usize>,
}

impl CompiledGraph {
    pub fn nodes(&self) -> &[CompiledNode] {
        &self.nodes
    }
    pub fn order(&self) -> &[usize] {
        &self.order
    }
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

pub fn compile(graph: &Graph) -> Result<CompiledGraph, CompileError> {
    if graph.nodes.is_empty() {
        return Err(CompileError::EmptyGraph);
    }

    let mut index: HashMap<&NodeId, usize> = HashMap::new();
    for (i, n) in graph.nodes.iter().enumerate() {
        if index.insert(&n.id, i).is_some() {
            return Err(CompileError::DuplicateNode(n.id.clone()));
        }
    }

    let mut nodes = Vec::with_capacity(graph.nodes.len());
    for n in &graph.nodes {
        match &n.spec {
            NodeSpec::Lfo { freq_hz, .. } => {
                if !freq_hz.is_finite() || *freq_hz <= 0.0 {
                    return Err(CompileError::InvalidLfoFreq {
                        node: n.id.clone(),
                        freq: *freq_hz,
                    });
                }
            }
            NodeSpec::SampleHold { rate_hz, .. } => {
                if !rate_hz.is_finite() || *rate_hz <= 0.0 {
                    return Err(CompileError::InvalidSampleHoldRate {
                        node: n.id.clone(),
                        rate: *rate_hz,
                    });
                }
            }
            NodeSpec::StepSequence { rate_hz, values } => {
                if !rate_hz.is_finite() || *rate_hz <= 0.0 {
                    return Err(CompileError::InvalidStepSequenceRate {
                        node: n.id.clone(),
                        rate: *rate_hz,
                    });
                }
                if values.is_empty() {
                    return Err(CompileError::EmptyStepSequence { node: n.id.clone() });
                }
                if values.len() > MAX_STEP_SEQUENCE_VALUES {
                    return Err(CompileError::StepSequenceTooLong {
                        node: n.id.clone(),
                        count: values.len(),
                        max: MAX_STEP_SEQUENCE_VALUES,
                    });
                }
                if let Some(index) = values.iter().position(|value| !value.is_finite()) {
                    return Err(CompileError::InvalidStepSequenceValue {
                        node: n.id.clone(),
                        index,
                    });
                }
            }
            NodeSpec::Slew {
                rise_per_second,
                fall_per_second,
            } => {
                if !rise_per_second.is_finite()
                    || *rise_per_second <= 0.0
                    || !fall_per_second.is_finite()
                    || *fall_per_second <= 0.0
                {
                    return Err(CompileError::InvalidSlewRate {
                        node: n.id.clone(),
                        rise: *rise_per_second,
                        fall: *fall_per_second,
                    });
                }
            }
            NodeSpec::Clamp { min, max } => {
                if !min.is_finite() || !max.is_finite() || min >= max {
                    return Err(CompileError::InvalidClampRange {
                        node: n.id.clone(),
                        min: *min,
                        max: *max,
                    });
                }
            }
            NodeSpec::Curve { exponent } => {
                if !exponent.is_finite() || *exponent <= 0.0 {
                    return Err(CompileError::InvalidCurveExponent {
                        node: n.id.clone(),
                        exponent: *exponent,
                    });
                }
            }
            NodeSpec::Quantize { steps } => {
                if *steps < 2 {
                    return Err(CompileError::InvalidQuantizeSteps {
                        node: n.id.clone(),
                        steps: *steps,
                    });
                }
            }
            NodeSpec::Math { operation, operand } => {
                if !operand.is_finite() {
                    return Err(CompileError::InvalidMathOperand {
                        node: n.id.clone(),
                        operand: *operand,
                    });
                }
                if matches!(operation, MathOperation::Divide) && *operand == 0.0 {
                    return Err(CompileError::DivisionByZero { node: n.id.clone() });
                }
            }
            NodeSpec::Envelope {
                attack_seconds,
                decay_seconds,
                sustain,
                release_seconds,
                ..
            } => {
                if !attack_seconds.is_finite()
                    || *attack_seconds < 0.0
                    || !decay_seconds.is_finite()
                    || *decay_seconds < 0.0
                    || !sustain.is_finite()
                    || !(0.0..=1.0).contains(sustain)
                    || !release_seconds.is_finite()
                    || *release_seconds < 0.0
                {
                    return Err(CompileError::InvalidEnvelope {
                        node: n.id.clone(),
                        attack: *attack_seconds,
                        decay: *decay_seconds,
                        sustain: *sustain,
                        release: *release_seconds,
                    });
                }
            }
            NodeSpec::Macro { binding, default } => {
                if binding.as_str().trim().is_empty() {
                    return Err(CompileError::EmptyBinding { node: n.id.clone() });
                }
                if !default.is_finite() {
                    return Err(CompileError::InvalidExternalDefault {
                        node: n.id.clone(),
                        value: *default,
                    });
                }
            }
            NodeSpec::Scale { min, max } => {
                if !min.is_finite() || !max.is_finite() || min >= max {
                    return Err(CompileError::InvalidScaleRange {
                        node: n.id.clone(),
                        min: *min,
                        max: *max,
                    });
                }
            }
            NodeSpec::Sink { .. } => {}
            NodeSpec::ExternalSource { binding, default } => {
                if binding.as_str().trim().is_empty() {
                    return Err(CompileError::EmptyBinding { node: n.id.clone() });
                }
                if !default.is_finite() {
                    return Err(CompileError::InvalidExternalDefault {
                        node: n.id.clone(),
                        value: *default,
                    });
                }
            }
            NodeSpec::ExternalSink { binding } => {
                if binding.as_str().trim().is_empty() {
                    return Err(CompileError::EmptyBinding { node: n.id.clone() });
                }
            }
            NodeSpec::Takeover { pickup_epsilon, .. } => {
                if !pickup_epsilon.is_finite() || *pickup_epsilon < 0.0 || *pickup_epsilon > 1.0 {
                    return Err(CompileError::InvalidPickupEpsilon {
                        node: n.id.clone(),
                        value: *pickup_epsilon,
                    });
                }
            }
        }

        let input = match &n.input {
            None => None,
            Some(src) => Some(*index.get(src).ok_or_else(|| CompileError::UnknownInput {
                node: n.id.clone(),
                input: src.clone(),
            })?),
        };

        let control =
            match &n.spec {
                NodeSpec::Takeover { control, .. } => Some(*index.get(control).ok_or_else(
                    || CompileError::UnknownControl {
                        node: n.id.clone(),
                        control: control.clone(),
                    },
                )?),
                NodeSpec::Envelope { gate, .. } => {
                    Some(*index.get(gate).ok_or_else(|| CompileError::UnknownGate {
                        node: n.id.clone(),
                        gate: gate.clone(),
                    })?)
                }
                _ => None,
            };

        nodes.push(CompiledNode {
            spec: n.spec.clone(),
            input,
            control,
        });
    }

    let order = topo_order(&nodes, &graph.nodes)?;
    Ok(CompiledGraph { nodes, order })
}

/// Kahn topological sort over primary inputs and auxiliary control inputs.
fn topo_order(
    nodes: &[CompiledNode],
    original: &[crate::graph::GraphNode],
) -> Result<Vec<usize>, CompileError> {
    let mut indegree = vec![0usize; nodes.len()];
    let mut dependents = vec![Vec::new(); nodes.len()];
    for (node_idx, node) in nodes.iter().enumerate() {
        for dependency in [node.input, node.control].into_iter().flatten() {
            indegree[node_idx] = indegree[node_idx].saturating_add(1);
            if let Some(edges) = dependents.get_mut(dependency) {
                edges.push(node_idx);
            }
        }
    }
    let mut ready: VecDeque<usize> = indegree
        .iter()
        .enumerate()
        .filter_map(|(idx, &degree)| (degree == 0).then_some(idx))
        .collect();
    let mut order = Vec::with_capacity(nodes.len());
    while let Some(node_idx) = ready.pop_front() {
        order.push(node_idx);
        for &dependent in dependents.get(node_idx).into_iter().flatten() {
            if let Some(degree) = indegree.get_mut(dependent) {
                *degree = degree.saturating_sub(1);
                if *degree == 0 {
                    ready.push_back(dependent);
                }
            }
        }
    }
    if order.len() != nodes.len() {
        let cycle = indegree
            .iter()
            .position(|&degree| degree > 0)
            .and_then(|idx| original.get(idx))
            .map(|node| node.id.clone())
            .unwrap_or_else(|| NodeId::new("unknown"));
        return Err(CompileError::Cycle(cycle));
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BindingId;
    use crate::graph::{GraphNode, LfoShape};
    use crate::{DeviceId, ParamName};

    fn lfo(id: &str) -> GraphNode {
        GraphNode {
            id: NodeId::new(id),
            spec: NodeSpec::Lfo {
                shape: LfoShape::Triangle,
                freq_hz: 1.0,
            },
            input: None,
        }
    }

    fn scale(id: &str, input: &str) -> GraphNode {
        GraphNode {
            id: NodeId::new(id),
            spec: NodeSpec::Scale {
                min: 0.0,
                max: 10.0,
            },
            input: Some(NodeId::new(input)),
        }
    }

    fn sink(id: &str, input: &str) -> GraphNode {
        GraphNode {
            id: NodeId::new(id),
            spec: NodeSpec::Sink {
                device: DeviceId::new("peak"),
                param: ParamName::new("filter.cutoff"),
            },
            input: Some(NodeId::new(input)),
        }
    }

    #[test]
    fn compiles_chain_in_dependency_order() {
        // Deliberately authored out of order: sink, scale, lfo.
        let g = Graph {
            nodes: vec![sink("s", "sc"), scale("sc", "l"), lfo("l")],
        };
        let c = compile(&g).expect("should compile");
        assert_eq!(c.len(), 3);

        // The lfo (index 2) must be evaluated before scale (1) before sink (0).
        let pos = |idx: usize| c.order().iter().position(|&x| x == idx).unwrap();
        assert!(pos(2) < pos(1), "lfo must precede scale");
        assert!(pos(1) < pos(0), "scale must precede sink");
    }

    #[test]
    fn rejects_empty_graph() {
        assert_eq!(compile(&Graph::default()), Err(CompileError::EmptyGraph));
    }

    #[test]
    fn rejects_duplicate_node_id() {
        let g = Graph {
            nodes: vec![lfo("l"), lfo("l")],
        };
        assert_eq!(
            compile(&g),
            Err(CompileError::DuplicateNode(NodeId::new("l")))
        );
    }

    #[test]
    fn rejects_unknown_input() {
        let g = Graph {
            nodes: vec![scale("sc", "nope")],
        };
        assert_eq!(
            compile(&g),
            Err(CompileError::UnknownInput {
                node: NodeId::new("sc"),
                input: NodeId::new("nope"),
            })
        );
    }

    #[test]
    fn rejects_cycle() {
        let g = Graph {
            nodes: vec![scale("a", "b"), scale("b", "a")],
        };
        assert!(matches!(compile(&g), Err(CompileError::Cycle(_))));
    }

    #[test]
    fn rejects_self_loop() {
        // A node whose `input` is itself: the degenerate one-node cycle.
        let g = Graph {
            nodes: vec![scale("a", "a")],
        };
        assert_eq!(compile(&g), Err(CompileError::Cycle(NodeId::new("a"))));
    }

    #[test]
    fn compiles_fan_out_with_shared_upstream_before_both_dependents() {
        // Two sinks reading the same upstream lfo node.
        let g = Graph {
            nodes: vec![lfo("l"), sink("s1", "l"), sink("s2", "l")],
        };
        let c = compile(&g).expect("fan-out must compile");

        let order = c.order();
        assert_eq!(order.len(), 3, "every node must appear in the order");
        let mut sorted = order.to_vec();
        sorted.sort_unstable();
        assert_eq!(
            sorted,
            vec![0, 1, 2],
            "every node must appear exactly once, with no gaps"
        );

        let pos = |idx: usize| order.iter().position(|&x| x == idx).unwrap();
        assert!(pos(0) < pos(1), "shared upstream must precede first sink");
        assert!(pos(0) < pos(2), "shared upstream must precede second sink");
    }

    #[test]
    fn rejects_non_positive_lfo_freq() {
        let g = Graph {
            nodes: vec![GraphNode {
                id: NodeId::new("l"),
                spec: NodeSpec::Lfo {
                    shape: LfoShape::Triangle,
                    freq_hz: 0.0,
                },
                input: None,
            }],
        };
        assert!(matches!(
            compile(&g),
            Err(CompileError::InvalidLfoFreq { .. })
        ));
    }

    #[test]
    fn validates_sample_hold_rate() {
        let graph = Graph {
            nodes: vec![GraphNode {
                id: NodeId::new("random"),
                spec: NodeSpec::SampleHold {
                    rate_hz: 0.0,
                    seed: 42,
                },
                input: None,
            }],
        };
        assert!(matches!(
            compile(&graph),
            Err(CompileError::InvalidSampleHoldRate { .. })
        ));
    }

    #[test]
    fn validates_step_sequence_rate_length_and_values() {
        let node = |rate_hz, values| Graph {
            nodes: vec![GraphNode {
                id: NodeId::new("steps"),
                spec: NodeSpec::StepSequence { rate_hz, values },
                input: None,
            }],
        };
        assert!(matches!(
            compile(&node(0.0, vec![0.0])),
            Err(CompileError::InvalidStepSequenceRate { .. })
        ));
        assert!(matches!(
            compile(&node(1.0, vec![])),
            Err(CompileError::EmptyStepSequence { .. })
        ));
        assert!(matches!(
            compile(&node(1.0, vec![0.0; MAX_STEP_SEQUENCE_VALUES + 1])),
            Err(CompileError::StepSequenceTooLong { .. })
        ));
        assert!(matches!(
            compile(&node(1.0, vec![0.0, f32::NAN])),
            Err(CompileError::InvalidStepSequenceValue { index: 1, .. })
        ));
        assert!(compile(&node(2.0, vec![0.0, 0.5, 1.0])).is_ok());
    }

    #[test]
    fn validates_transform_parameters() {
        let graph = |spec| Graph {
            nodes: vec![GraphNode {
                id: NodeId::new("transform"),
                spec,
                input: None,
            }],
        };
        assert!(matches!(
            compile(&graph(NodeSpec::Slew {
                rise_per_second: 0.0,
                fall_per_second: 1.0,
            })),
            Err(CompileError::InvalidSlewRate { .. })
        ));
        assert!(matches!(
            compile(&graph(NodeSpec::Clamp { min: 1.0, max: 1.0 })),
            Err(CompileError::InvalidClampRange { .. })
        ));
        assert!(matches!(
            compile(&graph(NodeSpec::Curve { exponent: f32::NAN })),
            Err(CompileError::InvalidCurveExponent { .. })
        ));
        assert!(matches!(
            compile(&graph(NodeSpec::Quantize { steps: 1 })),
            Err(CompileError::InvalidQuantizeSteps { .. })
        ));
        assert!(matches!(
            compile(&graph(NodeSpec::Math {
                operation: MathOperation::Divide,
                operand: 0.0,
            })),
            Err(CompileError::DivisionByZero { .. })
        ));
        assert!(matches!(
            compile(&graph(NodeSpec::Math {
                operation: MathOperation::Add,
                operand: f32::INFINITY,
            })),
            Err(CompileError::InvalidMathOperand { .. })
        ));
    }

    #[test]
    fn envelope_validates_and_orders_its_gate_dependency() {
        let graph = Graph {
            nodes: vec![
                GraphNode {
                    id: NodeId::new("env"),
                    spec: NodeSpec::Envelope {
                        gate: NodeId::new("gate"),
                        attack_seconds: 0.1,
                        decay_seconds: 0.2,
                        sustain: 0.6,
                        release_seconds: 0.3,
                    },
                    input: None,
                },
                GraphNode {
                    id: NodeId::new("gate"),
                    spec: NodeSpec::Macro {
                        binding: BindingId::new("gate"),
                        default: 0.0,
                    },
                    input: None,
                },
            ],
        };
        let compiled = compile(&graph).unwrap();
        assert_eq!(compiled.nodes()[0].control, Some(1));
        assert_eq!(compiled.order(), &[1, 0]);

        let invalid = Graph {
            nodes: vec![GraphNode {
                id: NodeId::new("env"),
                spec: NodeSpec::Envelope {
                    gate: NodeId::new("missing"),
                    attack_seconds: -1.0,
                    decay_seconds: 0.0,
                    sustain: 2.0,
                    release_seconds: 0.0,
                },
                input: None,
            }],
        };
        assert!(matches!(
            compile(&invalid),
            Err(CompileError::InvalidEnvelope { .. })
        ));

        let unknown = Graph {
            nodes: vec![GraphNode {
                id: NodeId::new("env"),
                spec: NodeSpec::Envelope {
                    gate: NodeId::new("missing"),
                    attack_seconds: 0.1,
                    decay_seconds: 0.1,
                    sustain: 0.5,
                    release_seconds: 0.1,
                },
                input: None,
            }],
        };
        assert!(matches!(
            compile(&unknown),
            Err(CompileError::UnknownGate { .. })
        ));

        let cycle = Graph {
            nodes: vec![
                GraphNode {
                    id: NodeId::new("env"),
                    spec: NodeSpec::Envelope {
                        gate: NodeId::new("scaled"),
                        attack_seconds: 0.1,
                        decay_seconds: 0.1,
                        sustain: 0.5,
                        release_seconds: 0.1,
                    },
                    input: None,
                },
                scale("scaled", "env"),
            ],
        };
        assert!(matches!(compile(&cycle), Err(CompileError::Cycle(_))));
    }

    #[test]
    fn rejects_inverted_scale_range() {
        let g = Graph {
            nodes: vec![GraphNode {
                id: NodeId::new("sc"),
                spec: NodeSpec::Scale {
                    min: 10.0,
                    max: 1.0,
                },
                input: None,
            }],
        };
        assert!(matches!(
            compile(&g),
            Err(CompileError::InvalidScaleRange { .. })
        ));
    }

    #[test]
    fn compiled_graph_exposes_read_only_view() {
        let g = Graph {
            nodes: vec![lfo("l")],
        };
        let c = compile(&g).unwrap();
        assert_eq!(c.nodes().len(), 1);
        assert_eq!(c.order(), &[0]);
        assert!(!c.is_empty());
    }

    #[test]
    fn validates_external_bindings_and_defaults() {
        let empty = Graph {
            nodes: vec![GraphNode {
                id: NodeId::new("input"),
                spec: NodeSpec::ExternalSource {
                    binding: BindingId::new(""),
                    default: 0.0,
                },
                input: None,
            }],
        };
        assert!(matches!(
            compile(&empty),
            Err(CompileError::EmptyBinding { .. })
        ));

        let non_finite = Graph {
            nodes: vec![GraphNode {
                id: NodeId::new("input"),
                spec: NodeSpec::ExternalSource {
                    binding: BindingId::new("osc.in"),
                    default: f32::NAN,
                },
                input: None,
            }],
        };
        assert!(matches!(
            compile(&non_finite),
            Err(CompileError::InvalidExternalDefault { .. })
        ));
    }

    #[test]
    fn takeover_orders_both_dependencies_before_it() {
        let graph = Graph {
            nodes: vec![
                GraphNode {
                    id: NodeId::new("pickup"),
                    spec: NodeSpec::Takeover {
                        control: NodeId::new("knob"),
                        pickup_epsilon: 0.01,
                    },
                    input: Some(NodeId::new("modulation")),
                },
                lfo("modulation"),
                GraphNode {
                    id: NodeId::new("knob"),
                    spec: NodeSpec::ExternalSource {
                        binding: BindingId::new("s88.knob.1"),
                        default: 0.0,
                    },
                    input: None,
                },
            ],
        };
        let compiled = compile(&graph).expect("takeover graph must compile");
        let pos = |idx: usize| {
            compiled
                .order()
                .iter()
                .position(|&candidate| candidate == idx)
                .unwrap()
        };
        assert!(pos(1) < pos(0));
        assert!(pos(2) < pos(0));
        assert_eq!(compiled.nodes()[0].control, Some(2));
    }

    #[test]
    fn takeover_rejects_unknown_control_and_invalid_epsilon() {
        let unknown = Graph {
            nodes: vec![GraphNode {
                id: NodeId::new("pickup"),
                spec: NodeSpec::Takeover {
                    control: NodeId::new("missing"),
                    pickup_epsilon: 0.01,
                },
                input: None,
            }],
        };
        assert_eq!(
            compile(&unknown),
            Err(CompileError::UnknownControl {
                node: NodeId::new("pickup"),
                control: NodeId::new("missing"),
            })
        );

        let invalid = Graph {
            nodes: vec![
                lfo("control"),
                GraphNode {
                    id: NodeId::new("pickup"),
                    spec: NodeSpec::Takeover {
                        control: NodeId::new("control"),
                        pickup_epsilon: 1.1,
                    },
                    input: None,
                },
            ],
        };
        assert!(matches!(
            compile(&invalid),
            Err(CompileError::InvalidPickupEpsilon { .. })
        ));
    }

    #[test]
    fn takeover_control_edges_participate_in_cycle_detection() {
        let graph = Graph {
            nodes: vec![
                GraphNode {
                    id: NodeId::new("pickup"),
                    spec: NodeSpec::Takeover {
                        control: NodeId::new("scaled"),
                        pickup_epsilon: 0.01,
                    },
                    input: None,
                },
                scale("scaled", "pickup"),
            ],
        };
        assert!(matches!(compile(&graph), Err(CompileError::Cycle(_))));
    }
}
